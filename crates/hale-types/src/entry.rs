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
//! Lowering reads the entry (F.40 phase 3, L4): the `main locus` it
//! deploys and every comparison it makes against "the main locus" are
//! the row's entry, by identity, and the row's `fn main` column
//! ([`EntryRow::fn_main`]) is the body lowering emits as the process's
//! entry point. A seed whose only `main locus` is module-nested has no
//! entry, and the check refuses it ([`EntryRow::refused`],
//! `lowering_laws`), so no program reaches lowering with a root that is
//! not the entry. The checks still judge that refused `main`: the
//! placement table is seeded from [`EntryRow::root`], the entry or,
//! failing one, the refused `main` (GH #825).

use hale_graph::ids::SiteId;
use hale_syntax::ast::{FnDecl, LocusDecl, TopDecl};
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

    /// The modules that enclose the declaration, outermost first: empty
    /// at the top level.
    pub fn modules<'b>(&self, bundle: &Bundle<'b>) -> Vec<&'b str> {
        let (program, path) = &self.at;
        let mut out = Vec::new();
        let Some(mut items) = bundle.programs.get(program).map(|p| &p.items[..]) else { return out };
        for i in path.split_last().map_or(&[][..], |(_, modules)| modules) {
            let Some(TopDecl::Module(m)) = items.get(*i) else { break };
            out.push(m.name.name.as_str());
            items = &m.items;
        }
        out
    }
}

/// The seed's `fn main`: the top-level one, the process's entry point
/// (GH #911: a `fn main` inside a module is not, and is refused).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnMain {
    /// The declaration's site, as the snapshot minted it. `None` only on
    /// a bundle nothing minted.
    pub site: Option<SiteId>,
    pub span: Span,
    /// The bundle program that holds it, and its item index.
    at: (String, usize),
}

impl FnMain {
    /// The declaration this row names, in the bundle it was read from.
    pub fn decl<'b>(&self, bundle: &Bundle<'b>) -> Option<&'b FnDecl> {
        match bundle.programs.get(&self.at.0)?.items.get(self.at.1)? {
            TopDecl::Fn(f) => Some(f),
            _ => None,
        }
    }
}

/// The process's entry point among `items`, a program's top-level
/// declarations: the first `fn main` at the top level. One definition,
/// the producer's; lowering applies it itself only to a view that
/// carries no row (a bare program's, which only tests lower).
pub fn top_level_fn_main(items: &[TopDecl]) -> Option<(usize, &FnDecl)> {
    items.iter().enumerate().find_map(|(i, item)| match item {
        TopDecl::Fn(f) if f.name.name == "main" => Some((i, f)),
        _ => None,
    })
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
    /// The seed's top-level `fn main`, which lowering emits as the
    /// process's entry point and whose body is where `return` exits the
    /// process (lowering's `in_main`). `None` for a seed with none (a
    /// library, an export-only wasm module). With more than one (the
    /// duplicate-name error), the first in program order.
    pub fn_main: Option<FnMain>,
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

    /// The module-nested `main locus` of a seed with no entry for that
    /// reason (decision 2): the first of the seed's own, which the check
    /// refuses at its name, since nothing else in the seed is the entry
    /// (`lowering_laws`, F.40 phase 3, L4). `None` whenever there is an
    /// entry: a module-nested `main` beside one is not deployed and is
    /// not refused for it (rule 1 counts it).
    pub fn refused(&self) -> Option<&MainLocus> {
        match self.entry {
            Err(NoEntry::OnlyModuleNested) => self.own().next(),
            _ => None,
        }
    }

    /// The `main locus` the placement table is seeded from: the entry,
    /// else the refused module-nested one ([`EntryRow::refused`]), so the
    /// rules that read the table still judge it as they judge a top-level
    /// one (GH #825). A program that reaches lowering has no refused
    /// `main`, so there it is the entry, which lowering deploys.
    pub fn root(&self) -> Option<&MainLocus> {
        self.entry().or_else(|| self.refused())
    }

    /// The world tier: the `main locus` declarations whose inline
    /// `claims { }` are the bundle's world law. Every one the bundle
    /// declares, in the witness's order: the entry's, a module-nested
    /// one's, and an imported application's, whose inline claims travel
    /// with it and are re-evaluated in the closing world (GH #733), with
    /// the importer's own `main locus` beside it or without one. The
    /// world is wider than the entry on purpose: decisions 1 and 2 say
    /// which declaration the seed RUNS, and an application's law does
    /// not stop binding because another seed runs it.
    pub fn world(&self) -> impl Iterator<Item = &MainLocus> {
        self.mains.iter()
    }

    /// Whether the bundle closes a world: some `main locus` states world
    /// law in it, so a top-level `claims { }` block of the closing seed's
    /// own is refused (the library tier is for a seed that closes none).
    pub fn closes_a_world(&self) -> bool {
        !self.mains.is_empty()
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

/// The root's declaration in `programs` ([`EntryRow::root`]: the entry,
/// which lowering deploys, else a refused module-nested `main`), by
/// [`entry_row_in`].
pub fn root_decl<'p>(programs: &[&'p hale_syntax::ast::Program]) -> Option<&'p LocusDecl> {
    let row = entry_row_in(programs);
    let (at, path) = row.root()?.index_in()?;
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
    // An imported seed's `fn main` is renamed with its seed, so the name
    // is the seed's own.
    let fn_main = bundle.programs.iter().find_map(|(name, program)| {
        top_level_fn_main(&program.items).map(|(i, f)| FnMain {
            site: bundle.snapshot.site_id(f.id),
            span: f.span,
            at: (name.to_string(), i),
        })
    });
    EntryRow { mains, entry, fn_main }
}
