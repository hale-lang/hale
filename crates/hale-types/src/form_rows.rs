//! The form rows (F.40 phase 3, C1): one resolved row per `@form`
//! declaration, with how its author configured its `sync` discipline
//! and the discipline it gets, kept apart.
//!
//! Before the rows the discipline had three readers that disagreed.
//! Sync inference wrote its pick into the AST as a `sync = <mode>`
//! argument (`apply_sync_inference`, before the desugar sequence), so
//! lowering and the checker read an argument the author never wrote;
//! the checker's cross-pool exemption admitted `serialized`, `striped`
//! and `lockfree`; inference's own "already configured" test counted
//! any `sync =` argument, `sync = none` included. The row says each
//! fact once, and the two questions the readers ask are two queries
//! over it:
//!
//! - [`FormRow::explicitly_configured`]: the author wrote a `sync =`
//!   argument, whatever it names. Inference runs only where this is
//!   false, so an explicit `sync = none` keeps its form unsynchronized.
//! - [`FormRow::safe_for_cross_domain_access`]: the discipline the
//!   form gets synchronizes access from another domain — `serialized`,
//!   `striped` or `lockfree`, written or inferred. `sync = none`, an
//!   argument that names no discipline, and an omitted argument
//!   inference left unsynchronized are not.
//!
//! The rows are the `sync_inference` family's: the snapshot demands
//! them after the mint (`Snapshot::demand_forms`), over its scope and
//! its placement table, and hands them to the checker (with its effects
//! certificate engine), to the model and to lowering, which lays each
//! map out by its effective discipline. Nothing writes the discipline
//! into the program. A bundle no snapshot holds (the checker's and the
//! model's test entries) builds them once, with [`form_rows`].

use std::collections::BTreeMap;

use hale_syntax::ast::{Expr, FormAnnotation, Literal, LocusDecl, NodeId, TopDecl};

use crate::placement::PlacementTable;
use crate::resolve::TopScope;
use crate::sync_inference::{InferredSync, SyncDiscipline};
use crate::Bundle;

/// A `sync` discipline a form can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Discipline {
    /// No synchronization: the form is touched from one domain.
    None,
    /// One mutex per map (F.32-1α).
    Serialized,
    /// Cell-level CAS with a grow lock (F.32-1β).
    Striped,
    /// Lock-free cells (F.32-1γ).
    Lockfree,
}

impl Discipline {
    /// The discipline a `sync = <name>` argument spells, if it spells one.
    pub fn from_label(label: &str) -> Option<Discipline> {
        match label {
            "none" => Some(Discipline::None),
            "serialized" => Some(Discipline::Serialized),
            "striped" => Some(Discipline::Striped),
            "lockfree" => Some(Discipline::Lockfree),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Discipline::None => "none",
            Discipline::Serialized => "serialized",
            Discipline::Striped => "striped",
            Discipline::Lockfree => "lockfree",
        }
    }

    /// Whether the discipline orders access from more than one domain.
    pub fn synchronizes(self) -> bool {
        self != Discipline::None
    }
}

/// How the author configured a form's `sync` discipline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncConfig {
    /// No `sync =` argument: the discipline is inference's.
    Omitted,
    /// `sync = <discipline>`, `none` included.
    Explicit(Discipline),
    /// A `sync =` argument that names no discipline (`sync = fast`,
    /// `sync = 3`). It is configuration — inference does not run — and
    /// it synchronizes nothing; the form check reports it.
    Invalid,
}

/// One `@form` declaration's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormRow {
    /// The declaration's name, as the checker and inference key loci.
    pub locus: String,
    /// The declaration's identity, as the snapshot minted it: what
    /// lowering finds the row by, a monomorph by its template's.
    /// `NodeId::NONE` on a bundle nothing minted.
    pub id: NodeId,
    /// The form's kind: `hashmap`, `vec`, `ring_buffer`, ...
    pub form: String,
    /// Declared inside a `module { }`, at any depth. Inference reads the
    /// top level only, as it always has.
    pub module_nested: bool,
    pub config: SyncConfig,
    /// The discipline the form gets: the written one, else inference's
    /// pick, else none.
    pub effective: Discipline,
    /// Inference's reasoning, for a `hashmap` form the author did not
    /// configure: the pools it observed and whether a mutate is hot.
    pub inferred: Option<InferredSync>,
    /// The form's fixed capacity, its written `cap = N` (a positive
    /// integer literal): a ring buffer's or an LRU cache's, and a
    /// lockfree map's. Lowering lays the slot out by it; the form check
    /// reports a form that needs one and has none.
    pub cap: Option<u64>,
}

impl FormRow {
    /// The author wrote a `sync =` argument. Inference runs only where
    /// this is false.
    pub fn explicitly_configured(&self) -> bool {
        self.config != SyncConfig::Omitted
    }

    /// The discipline the form gets synchronizes access from another
    /// domain: `serialized`, `striped` or `lockfree`, written or
    /// inferred. An explicit `sync = none` is configured and not safe.
    pub fn safe_for_cross_domain_access(&self) -> bool {
        self.effective.synchronizes()
    }
}

/// The rows of a bundle, one per `@form` declaration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormRows {
    rows: Vec<FormRow>,
    by_name: BTreeMap<String, usize>,
    /// Keyed by the id's number: `NodeId`'s equality ignores the id.
    by_id: BTreeMap<u32, usize>,
}

impl FormRows {
    pub fn rows(&self) -> &[FormRow] {
        &self.rows
    }

    /// The row of the declaration named `locus`. Two declarations of
    /// one name (in two modules) share the first one's row, as the
    /// name-keyed readers always have.
    pub fn named(&self, locus: &str) -> Option<&FormRow> {
        self.by_name.get(locus).map(|&i| &self.rows[i])
    }

    /// The row of declaration `l`: by its identity when it has one (a
    /// monomorph carries its template's), else by its name.
    pub fn of(&self, l: &LocusDecl) -> Option<&FormRow> {
        if !l.id.is_none() {
            if let Some(&i) = self.by_id.get(&l.id.0) {
                return Some(&self.rows[i]);
            }
        }
        self.named(&l.name.name)
    }

    /// The discipline declaration `l` gets: its row's effective one. A
    /// declaration with no row (one no snapshot inferred over: the
    /// stdlib's, merged into the program lowering walks) gets its
    /// written configuration's.
    pub fn effective(&self, l: &LocusDecl) -> Discipline {
        match self.of(l) {
            Some(row) => row.effective,
            None => match l.form.as_ref().map(sync_config) {
                Some(SyncConfig::Explicit(d)) => d,
                _ => Discipline::None,
            },
        }
    }

    /// Whether `l`'s form is safe for cross-domain access, for the
    /// readers that ask it as one question: the model's `sync_form`
    /// (the `depends` law), the effects certificate engine (a call into
    /// the form, or into a locus holding it, can take its lock) and the
    /// checker's instance-aliasing rule. Each asks
    /// [`FormRow::safe_for_cross_domain_access`] alone: an explicit
    /// `sync = none` takes no lock and is not one. A declaration with
    /// no row reads its written argument.
    pub fn synchronizes(&self, l: &LocusDecl) -> bool {
        l.form.is_some() && self.effective(l).synchronizes()
    }

    /// Declaration `l`'s fixed capacity: its row's `cap`. A declaration
    /// with no row reads its written argument, as [`FormRows::effective`]
    /// does.
    pub fn cap(&self, l: &LocusDecl) -> Option<u64> {
        match self.of(l) {
            Some(row) => row.cap,
            None => l.form.as_ref().and_then(form_cap),
        }
    }

    fn push(&mut self, row: FormRow) {
        let i = self.rows.len();
        self.by_name.entry(row.locus.clone()).or_insert(i);
        if !row.id.is_none() {
            self.by_id.entry(row.id.0).or_insert(i);
        }
        self.rows.push(row);
    }

    /// The rows with no inference: every declaration's written
    /// configuration, and the discipline it alone gives. What a
    /// declaration nothing inferred over reads (the bundled stdlib's,
    /// which no user call site configures).
    pub fn configured<'a>(items: impl IntoIterator<Item = &'a TopDecl>) -> FormRows {
        fn walk(items: &[TopDecl], module_nested: bool, rows: &mut FormRows) {
            for item in items {
                match item {
                    TopDecl::Module(m) => walk(&m.items, true, rows),
                    TopDecl::Locus(l) => {
                        let Some(form) = &l.form else { continue };
                        let config = sync_config(form);
                        rows.push(FormRow {
                            locus: l.name.name.clone(),
                            id: l.id,
                            form: form.name.name.clone(),
                            module_nested,
                            config,
                            effective: match config {
                                SyncConfig::Explicit(d) => d,
                                SyncConfig::Omitted | SyncConfig::Invalid => Discipline::None,
                            },
                            inferred: None,
                            cap: form_cap(form),
                        });
                    }
                    _ => {}
                }
            }
        }
        let mut rows = FormRows::default();
        for item in items {
            walk(std::slice::from_ref(item), false, &mut rows);
        }
        rows
    }

    /// These rows and `more`'s, these first: a declaration both hold
    /// (by identity, or by name where `more`'s row has none) keeps this
    /// side's row.
    pub fn extended(mut self, more: FormRows) -> FormRows {
        for row in more.rows {
            let held = if row.id.is_none() {
                self.by_name.contains_key(&row.locus)
            } else {
                self.by_id.contains_key(&row.id.0)
            };
            if !held {
                self.push(row);
            }
        }
        self
    }
}

/// How `form` configures its `sync` discipline: the first `sync =`
/// argument, as every reader took it.
pub fn sync_config(form: &FormAnnotation) -> SyncConfig {
    match form.args.iter().find(|a| a.name.name == "sync") {
        None => SyncConfig::Omitted,
        Some(arg) => match &arg.value {
            Expr::Ident(i) => Discipline::from_label(&i.name).map_or(SyncConfig::Invalid, SyncConfig::Explicit),
            _ => SyncConfig::Invalid,
        },
    }
}

/// `form`'s fixed capacity: its first `cap =` argument when that is a
/// positive integer literal.
fn form_cap(form: &FormAnnotation) -> Option<u64> {
    form.args.iter().find(|a| a.name.name == "cap").and_then(|a| match &a.value {
        Expr::Literal(Literal::Int(n), _) if *n > 0 => Some(*n as u64),
        _ => None,
    })
}

/// The producer: every `@form` declaration of `bundle` with its written
/// configuration, and for a `hashmap` form its author did not configure,
/// the discipline sync inference picks from the domains each of its
/// instances is accessed from (the placement table's, per instance).
///
/// `resolved` is whether `top` was built without a diagnostic. Inference
/// reads declarations through the scope, and over a scope that did not
/// resolve it does not run: such a program does not build, and the rows
/// carry only what was written.
pub fn form_rows(bundle: &Bundle<'_>, top: &TopScope, placement: &PlacementTable, resolved: bool) -> FormRows {
    let mut rows = FormRows::configured(bundle.programs.values().flat_map(|p| p.items.iter()));
    if !resolved {
        return rows;
    }
    let inferred = crate::sync_inference::infer_sync_for_bundle(bundle, top, placement, &rows);
    for row in &mut rows.rows {
        // Inference keys its candidates by name over the top level: a
        // module's form of the same name is not one of them.
        if row.config != SyncConfig::Omitted || row.module_nested {
            continue;
        }
        let Some(inf) = inferred.get(&row.locus) else { continue };
        row.effective = match inf.discipline {
            SyncDiscipline::None => Discipline::None,
            SyncDiscipline::Serialized => Discipline::Serialized,
            SyncDiscipline::Striped => Discipline::Striped,
        };
        row.inferred = Some(inf.clone());
    }
    rows
}
