//! The `entrypoint` family (F.40 phase 3, E0): which locus is the
//! program's `main`, by identity, as one row.
//!
//! Before this row the entry had twelve definitions that disagreed on
//! which `main locus` a seed runs. The row settles it, and it is the
//! family's invariant (`spec/registry.md`, `entrypoint`): **an imported
//! `main` is not the entry.** A library's `main locus` is a declaration
//! the importing seed does not run; the entry is the importing seed's
//! own. A seed whose only `main` is imported has no entry.
//!
//! The row keeps every `main locus` the bundle declares, each with the
//! fact the decision reads ([`MainLocus::imported`]): the witness of why
//! a declaration is or is not the entry, and what the one-main rule
//! (rule 1) counts.
//!
//! Lowering reads the entry (F.40 phase 3, L4): the `main locus` it
//! deploys and every comparison it makes against "the main locus" are
//! the row's entry, by identity, and the row's `fn main` column
//! ([`EntryRow::fn_main`]) is the body lowering emits as the process's
//! entry point.

use hale_graph::ids::SiteId;
use hale_syntax::ast::{FnDecl, LocusDecl, TopDecl};
use hale_syntax::{Diag, Span};

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
    /// The bundle program that holds it, and its item index: what
    /// [`MainLocus::decl`] follows back to the declaration.
    at: (String, usize),
}

/// The locus declaration at item `index` of `items`.
fn locus_at(items: &[TopDecl], index: usize) -> Option<&LocusDecl> {
    match items.get(index)? {
        TopDecl::Locus(l) => Some(l),
        _ => None,
    }
}

impl MainLocus {
    /// The declaration this row names, in the bundle it was read from.
    pub fn decl<'b>(&self, bundle: &Bundle<'b>) -> Option<&'b LocusDecl> {
        let (program, index) = &self.at;
        locus_at(&bundle.programs.get(program)?.items, *index)
    }

    /// Where this declaration is in the programs a row built by
    /// [`entry_row_in`] read: the program's index and the declaration's
    /// item index in it. `None` for a bundle's row, whose programs are
    /// named by path.
    pub fn index_in(&self) -> Option<(usize, usize)> {
        Some((self.at.0.parse().ok()?, self.at.1))
    }

    /// The name of the bundle program that holds it: a snapshot's
    /// bundle names a program by its path.
    pub fn program(&self) -> &str {
        &self.at.0
    }
}

/// The refusal of a seed whose own `main locus` nothing starts: the
/// seed has no top-level `fn main` to instantiate it (`spec/semantics.md`
/// § "The entry locus"). One wording, which the build rules beside the
/// check report ([`unstarted_main_diags`]) and lowering keeps as its
/// backstop.
pub fn unstarted_main_message(locus: &str) -> String {
    format!("a `main locus` is started by `fn main`, and this seed has none: add `fn main() {{ {locus} {{ }}; }}`")
}

/// GH #2108: [`unstarted_main_locus`]'s refusal, as the error a build
/// rule reports (`crate::build_rule_diags`): `hale check`, the editor
/// and every build path report it, at the locus's name.
pub fn unstarted_main_diags(bundle: &Bundle<'_>, row: &EntryRow) -> Vec<Diag> {
    unstarted_main_locus(bundle, row).map(|(decl, message)| Diag::ty(decl.name.span, message)).into_iter().collect()
}

/// GH #2108: the seed's own `main locus`, when nothing in the seed starts
/// it, with where to report it (its name). A `main locus` does not start
/// itself: the seed's top-level `fn main` instantiates it, and a seed
/// that declares its own and no `fn main` has nothing to run, so `hale
/// build`, `hale run` and `hale test` refuse it; `hale check` refuses it
/// too, at the same name, so a seed the check accepts builds.
///
/// `None` when the seed has a `fn main` (the row's column), when its only
/// `main locus` is imported (a library's, which the importer starts), and
/// for the two programs whose entry is not their own `fn main`:
///
/// - an `@export` module, the wasm entry inversion's (an export-only
///   module needs no `fn main`; a native target refuses the inversion
///   itself, by the admission law); and
/// - a `*_bench.hl` file, which must declare no `fn main`: `hale bench`
///   appends the driver's.
///
/// With more than one own `main locus` (rule 1's error), the first in the
/// bundle's order, as lowering names it.
pub fn unstarted_main_locus<'b>(bundle: &Bundle<'b>, row: &EntryRow) -> Option<(&'b LocusDecl, String)> {
    if row.fn_main.is_some() {
        return None;
    }
    let exports = bundle.programs.values().flat_map(|p| p.items.iter()).any(|item| match item {
        TopDecl::Fn(f) => f.export,
        TopDecl::Locus(l) => l.export,
        _ => false,
    });
    if exports {
        return None;
    }
    let first = row.own().next()?;
    if first.program().ends_with("_bench.hl") {
        return None;
    }
    let decl = first.decl(bundle)?;
    Some((decl, unstarted_main_message(&decl.name.name)))
}

/// The seed's `fn main`: the top-level one, the process's entry point.
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
    /// Every `main locus` came in through an `import`.
    OnlyImported,
}

/// The `entrypoint` row of one bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryRow {
    /// Every `main locus` the bundle declares, in the bundle's program
    /// order and each program's declaration order: the witness.
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
    /// The entry: the seed's own `main locus`. With more than
    /// one (rule 1's error), the last, as the checker's pool map always
    /// took it.
    pub fn entry(&self) -> Option<&MainLocus> {
        self.entry.ok().map(|i| &self.mains[i])
    }

    /// Why there is no entry, when there is none.
    pub fn no_entry(&self) -> Option<NoEntry> {
        self.entry.err()
    }

    /// The seed's own `main locus` declarations, the ones the entry is
    /// chosen from: what rule 1 counts. One in a program rule 1 accepts.
    pub fn own(&self) -> impl Iterator<Item = &MainLocus> {
        self.mains.iter().filter(|m| !m.imported)
    }

    /// The world tier: the `main locus` declarations whose inline
    /// `claims { }` are the bundle's world law. Every one the bundle
    /// declares, in the witness's order: the entry's and an imported
    /// application's, whose inline claims travel with it and are
    /// re-evaluated in the closing world (GH #733), with the importer's
    /// own `main locus` beside it or without one. The world is wider than
    /// the entry on purpose: the entry says which declaration the seed
    /// RUNS, and an application's law does not stop binding because
    /// another seed runs it.
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
/// what lands in the entry lowering deploys, before the
/// mint writes there (an environment's constitutions, `--api`'s entry,
/// GH #1106's generated binding), and a reader handed programs and no
/// bundle reads there. An injection adds
/// members to a locus and top-level items after
/// the existing ones, never a `main locus`, so what it finds
/// is what the snapshot's row names after the mint.
pub fn entry_row_in(programs: &[&hale_syntax::ast::Program]) -> EntryRow {
    entry_row(&Bundle::new(programs.iter().enumerate().map(|(i, p)| (format!("{i:08}"), *p)).collect()))
}

/// The entry's declaration in `programs` ([`EntryRow::entry`], which
/// lowering deploys), by [`entry_row_in`].
pub fn entry_decl<'p>(programs: &[&'p hale_syntax::ast::Program]) -> Option<&'p LocusDecl> {
    let row = entry_row_in(programs);
    let (at, index) = row.entry()?.index_in()?;
    locus_at(&programs[at].items, index)
}

/// The `entrypoint` family's producer: one walk over the bundle's
/// declarations. It reads declarations only, so nothing the check
/// reports can block it; a snapshot demands it once
/// (`Snapshot::demand_entry`), and a bundle no snapshot holds (the
/// checker's test entries, sync inference's single-program bundle)
/// builds its own.
pub fn entry_row(bundle: &Bundle<'_>) -> EntryRow {
    let mut mains = Vec::new();
    for (name, program) in &bundle.programs {
        for (i, item) in program.items.iter().enumerate() {
            match item {
                TopDecl::Locus(l) if l.is_main => mains.push(MainLocus {
                    site: bundle.snapshot.site_id(l.id),
                    name: l.name.name.clone(),
                    span: l.span,
                    imported: l.imported,
                    at: (name.to_string(), i),
                }),
                _ => {}
            }
        }
    }
    let entry = match mains.iter().rposition(|m| !m.imported) {
        Some(i) => Ok(i),
        None if mains.is_empty() => Err(NoEntry::NoMain),
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
