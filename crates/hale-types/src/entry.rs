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
//!
//! Lowering does not read the row yet (F.40 phase 3, L4): it picks its
//! deployment root itself, nested `main`s included, and emits that
//! root's placement. Until it reads the entry, the row carries that
//! choice as a provisional column, [`EntryRow::lowering_root`], and
//! the placement-safety rules read the column, so the checker guards
//! the topology lowering emits; the decisions bind what reads the
//! entry.

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
        hale_syntax::ast::locus_at(&bundle.programs.get(program)?.items, path)
    }

    /// Where this declaration is in the programs a row built by
    /// [`entry_row_in`] read: the program's index and the declaration's
    /// path in it (for [`hale_syntax::ast::locus_at`]). `None` for a
    /// bundle's row, whose programs are named by path.
    pub fn index_in(&self) -> Option<(usize, &[usize])> {
        Some((self.at.0.parse().ok()?, &self.at.1))
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
    /// PROVISIONAL: the `main locus` lowering deploys as its root today,
    /// which is not always the entry. `collect_main_placement` takes the
    /// first `main locus` over the flat declarations whose name does not
    /// start with `__lib_`, module-nested ones included, and emits its
    /// placement, so a seed whose only `main` is module-nested has no
    /// entry and still a deployment root. The placement-safety rules
    /// (the F.31 pool map, the pinned-in-a-loop rule) read this column,
    /// not the entry, because they guard the threads lowering spawns.
    /// It is lowering's choice copied exactly, its name test included
    /// (the rename pass marks every `__lib_` name it gives `imported`,
    /// but a program handed in already renamed carries the name alone).
    /// The column goes when lowering reads the entry (F.40 phase 3, L4).
    pub lowering_root: Option<MainLocus>,
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

/// The row of programs no snapshot has minted yet, each program named by
/// its index in `programs`, so [`MainLocus::index_in`] says where a
/// declaration is. The producer's row, over the programs as they stand:
/// what lands in the entry, or in the root lowering deploys, before the
/// mint writes there (an environment's constitutions, `--api`'s entry,
/// GH #1106's generated binding), and a reader handed programs and no
/// bundle (the roles `--matrix` maps) reads there. An injection adds
/// members to a locus and top-level items after
/// the existing ones, never a `main locus` or a module, so what it finds
/// is what the snapshot's row names after the mint.
pub fn entry_row_in(programs: &[&hale_syntax::ast::Program]) -> EntryRow {
    entry_row(&Bundle::new(programs.iter().enumerate().map(|(i, p)| (format!("{i:08}"), *p)).collect()))
}

/// The lowering root's declaration in `programs`, by [`entry_row_in`].
pub fn lowering_root_decl<'p>(programs: &[&'p hale_syntax::ast::Program]) -> Option<&'p LocusDecl> {
    let row = entry_row_in(programs);
    let (at, path) = row.lowering_root.as_ref()?.index_in()?;
    hale_syntax::ast::locus_at(&programs[at].items, path)
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
    // The walk's order is `flat_decls`'s: a module's contents in its
    // place, as lowering reads the program.
    let lowering_root = mains.iter().find(|m| !m.name.starts_with("__lib_")).cloned();
    EntryRow { mains, entry, lowering_root }
}
