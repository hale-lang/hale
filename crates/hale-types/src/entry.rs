//! The `entrypoint` family (F.40 phase 3, E0): which locus is the
//! program's `main`, by identity, as one row.
//!
//! Before this row the entry had twelve definitions that disagreed on
//! two shapes. The row settles both, and they are the family's
//! invariants (`spec/registry.md`, `entrypoint`):
//!
//! 1. **An imported `main` is not the entry.** A library's `main
//!    locus` is a declaration the importing seed does not run; the
//!    entry is the importing seed's own. A seed whose only `main` is
//!    imported has no entry.
//! 2. **A module-nested `main` is not the entry.** The entry is a
//!    top-level `main locus` of the seed's own files, as a seed's
//!    entry point is its top-level `fn main` (`spec/semantics.md`
//!    § "Declarations inside `module { }`"), so rule 9's closed world
//!    is the top-level one. A seed whose only `main` is module-nested
//!    has no entry; a seed with both keeps the top-level one.
//!
//! The row keeps every `main locus` the bundle declares, each with the
//! two facts the decisions read ([`MainLocus::imported`],
//! [`MainLocus::module_nested`]): the witness of why a declaration is
//! or is not the entry, and what the one-main rule (rule 1) counts,
//! which a module-nested `main` still joins (GH #825).

use hale_graph::ids::SiteId;
use hale_syntax::ast::{LocusDecl, TopDecl};
use hale_syntax::Span;

use crate::Bundle;

/// One `main locus` declaration of the bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MainLocus {
    /// The declaration's site, as the snapshot minted it. `None` only on
    /// a bundle nothing minted (the checker's test entries).
    pub site: Option<SiteId>,
    pub name: String,
    pub span: Span,
    /// Merged from an imported seed: the cross-seed rename pass marks
    /// every declaration it renamed with its seed `imported` (GH #1104
    /// piece 5). The mark is the one definition; the `__lib_…` name it
    /// also gives is spelling, not a second test.
    pub imported: bool,
    /// Declared inside a `module { }`, at any depth.
    pub module_nested: bool,
    /// The bundle program that holds it, and the item index at each
    /// depth: what [`MainLocus::decl`] follows back to the declaration.
    at: (String, Vec<usize>),
}

impl MainLocus {
    /// The declaration this row names, in the bundle it was read from.
    pub fn decl<'b>(&self, bundle: &Bundle<'b>) -> Option<&'b LocusDecl> {
        let (program, path) = &self.at;
        let mut items: &'b [TopDecl] = &bundle.programs.get(program)?.items;
        let (last, modules) = path.split_last()?;
        for i in modules {
            let TopDecl::Module(m) = items.get(*i)? else { return None };
            items = &m.items;
        }
        match items.get(*last)? {
            TopDecl::Locus(l) => Some(l),
            _ => None,
        }
    }

    /// Whether the decisions let this declaration be the entry: the
    /// seed's own, at the top level.
    pub fn may_be_the_entry(&self) -> bool {
        !self.imported && !self.module_nested
    }
}

/// Why a bundle has no entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoEntry {
    /// No `main locus` at all: a library, or a bare `fn main` script.
    NoMain,
    /// Every `main locus` came in through an `import` (decision 1).
    OnlyImported,
    /// The seed's own `main locus` is inside a `module { }` (decision 2).
    OnlyModuleNested,
}

/// The `entrypoint` row of one bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryRow {
    /// Every `main locus` the bundle declares, in the bundle's program
    /// order and each program's declaration order (a module's contents
    /// after the module): the witness.
    pub mains: Vec<MainLocus>,
    entry: Result<usize, NoEntry>,
}

impl EntryRow {
    /// The entry: the seed's own top-level `main locus`. With more than
    /// one (rule 1's error), the last, as the checker's pool map always
    /// took it.
    pub fn entry(&self) -> Option<&MainLocus> {
        self.entry.ok().map(|i| &self.mains[i])
    }

    /// Why there is no entry, when there is none.
    pub fn no_entry(&self) -> Option<NoEntry> {
        self.entry.err()
    }

    /// The `main locus` declarations the entry is chosen from: the
    /// seed's own, at the top level. One in a program rule 1 accepts.
    pub fn candidates(&self) -> impl Iterator<Item = &MainLocus> {
        self.mains.iter().filter(|m| m.may_be_the_entry())
    }

    /// The seed's own `main locus` declarations, module-nested ones
    /// included: what rule 1 counts.
    pub fn own(&self) -> impl Iterator<Item = &MainLocus> {
        self.mains.iter().filter(|m| !m.imported)
    }
}

/// The `entrypoint` family's producer: one walk over the bundle's
/// declarations. It reads declarations only, so nothing the check
/// reports can block it; a snapshot demands it once
/// (`Snapshot::demand_entry`), and a bundle no snapshot holds (the
/// checker's test entries, sync inference's single-program bundle)
/// builds its own.
pub fn entry_row(bundle: &Bundle<'_>) -> EntryRow {
    fn walk(
        items: &[TopDecl],
        program: &str,
        path: &mut Vec<usize>,
        bundle: &Bundle<'_>,
        out: &mut Vec<MainLocus>,
    ) {
        for (i, item) in items.iter().enumerate() {
            path.push(i);
            match item {
                TopDecl::Locus(l) if l.is_main => out.push(MainLocus {
                    site: bundle.snapshot.site_id(l.id),
                    name: l.name.name.clone(),
                    span: l.span,
                    imported: l.imported,
                    module_nested: path.len() > 1,
                    at: (program.to_string(), path.clone()),
                }),
                TopDecl::Module(m) => walk(&m.items, program, path, bundle, out),
                _ => {}
            }
            path.pop();
        }
    }
    let mut mains = Vec::new();
    for (name, program) in &bundle.programs {
        walk(&program.items, name, &mut Vec::new(), bundle, &mut mains);
    }
    let entry = match mains.iter().rposition(MainLocus::may_be_the_entry) {
        Some(i) => Ok(i),
        None if mains.is_empty() => Err(NoEntry::NoMain),
        None if mains.iter().any(|m| !m.imported) => Err(NoEntry::OnlyModuleNested),
        None => Err(NoEntry::OnlyImported),
    };
    EntryRow { mains, entry }
}
