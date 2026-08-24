//! #392 thread 1 — the normalized, provenance-bearing model.
//!
//! Every reviewer of the #382 stack independently converged on this
//! as the next architectural milestone: one typed model — sorts,
//! relations, labels, weights, per-edge source spans — derived once
//! and consumed by every judgment form. The relations themselves
//! already live in one place (`AllocSummary` for calls,
//! `BusGraph` for bus ends, both span-carrying since birth); what
//! was missing is what THIS module derives:
//!
//!  * **Declaration provenance** — which seed declared each decl,
//!    and where. Witnesses use it to say *where to edit* (the
//!    callsite that introduced the crossing edge, the destination's
//!    declaration) and to refuse to point a span into a source
//!    space the diag renderer can't map (stdlib bodies parse in
//!    their own offset space; a span from there attributed to a
//!    user file is a lie).
//!  * **The phase relation** — fn → phase, with lifecycle hooks
//!    (`birth`, `accept`, `release`, `run`, `drain`, `dissolve`)
//!    and modes (`bulk`, `harmonic`, `resolution`) distinguished
//!    from ordinary methods. `during P` evaluates against this
//!    relation, and the topology artifact exports it, which is what
//!    makes a `during` claim row independently re-derivable.
//!  * **The seed sort** — alias → member decls, from the bundle's
//!    import renames. `cover ... of <alias>` rows become
//!    re-derivable the same way.
//!
//! The model is DERIVED, never authored: `Model::derive` reads the
//! merged programs and the rename table, nothing else. It must stay
//! cheap — it is built once per law judgment and once per
//! artifact dump.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::*;
use hale_syntax::Span;

use crate::alloc_summary::FnKey;

/// Which compilation unit declared a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeclOrigin {
    /// The main seed — the bundle the user is checking. Spans from
    /// here are in the bundle's own offset space and safe to point
    /// diagnostics at.
    Main,
    /// An imported seed, by alias. Its decls arrive mangled
    /// (`__lib_<id>_<stem>_<name>`); spans are in the bundle space
    /// (the merge preserves them) and safe to point at.
    Seed(String),
}

/// A declared top-level name: where it came from and where it is.
#[derive(Debug, Clone)]
pub struct DeclInfo {
    pub origin: DeclOrigin,
    pub span: Span,
}

/// One row of the phase relation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseEntry {
    /// The phase name — the hook/mode/method name (`birth`, `run`,
    /// `plan`, …). `during P` matches against this.
    pub phase: String,
    /// True for lifecycle hooks and modes — the phases the RUNTIME
    /// drives — false for ordinary methods (a source-slice phase,
    /// per the shipped `during` doctrine).
    pub hook: bool,
}

/// The derived model: declaration provenance, the phase relation,
/// and the seed sort. Relations (calls, publishes, subscribes) stay
/// where they always were — `AllocSummary` and `BusGraph`, both
/// span-carrying — this is the half no single place held.
#[derive(Debug, Clone, Default)]
pub struct Model {
    /// Every top-level decl the bundle owns (loci, free fns, types,
    /// interfaces, groups) → origin + decl span. Keyed by the
    /// POST-MERGE name (mangled for imported decls), because that is
    /// the name every graph node carries.
    pub decls: BTreeMap<String, DeclInfo>,
    /// The phase relation over locus-owned fns.
    pub phases: BTreeMap<FnKey, PhaseEntry>,
    /// Seed sort: alias → the mangled names of its member decls.
    pub seeds: BTreeMap<String, BTreeSet<String>>,
}

impl Model {
    pub fn derive(
        programs: &[&Program],
        import_renames: &[(Vec<String>, String)],
    ) -> Model {
        let mut seeds: BTreeMap<String, BTreeSet<String>> =
            BTreeMap::new();
        let mut seed_of: BTreeMap<&str, &str> = BTreeMap::new();
        for (segs, mangled) in import_renames {
            let Some(alias) = segs.first() else { continue };
            seeds
                .entry(alias.clone())
                .or_default()
                .insert(mangled.clone());
            seed_of.insert(mangled, alias);
        }

        let mut decls: BTreeMap<String, DeclInfo> = BTreeMap::new();
        let mut phases: BTreeMap<FnKey, PhaseEntry> = BTreeMap::new();
        fn walk(
            items: &[TopDecl],
            seed_of: &BTreeMap<&str, &str>,
            decls: &mut BTreeMap<String, DeclInfo>,
            phases: &mut BTreeMap<FnKey, PhaseEntry>,
        ) {
            let origin = |name: &str| match seed_of.get(name) {
                Some(alias) => DeclOrigin::Seed(alias.to_string()),
                None => DeclOrigin::Main,
            };
            for item in items {
                match item {
                    TopDecl::Locus(l) => {
                        let locus = l.name.name.clone();
                        decls.insert(
                            locus.clone(),
                            DeclInfo {
                                origin: origin(&locus),
                                span: l.name.span,
                            },
                        );
                        for m in &l.members {
                            let (name, hook): (String, bool) = match m
                            {
                                LocusMember::Fn(fd) => {
                                    (fd.name.name.clone(), false)
                                }
                                LocusMember::Lifecycle(lc) => (
                                    match lc.kind {
                                        LifecycleKind::Birth => "birth",
                                        LifecycleKind::Accept => {
                                            "accept"
                                        }
                                        LifecycleKind::Release => {
                                            "release"
                                        }
                                        LifecycleKind::Run => "run",
                                        LifecycleKind::Drain => "drain",
                                        LifecycleKind::Dissolve => {
                                            "dissolve"
                                        }
                                    }
                                    .to_string(),
                                    true,
                                ),
                                LocusMember::Mode(md) => (
                                    match md.kind {
                                        ModeKind::Bulk => "bulk",
                                        ModeKind::Harmonic => {
                                            "harmonic"
                                        }
                                        ModeKind::Resolution => {
                                            "resolution"
                                        }
                                    }
                                    .to_string(),
                                    true,
                                ),
                                _ => continue,
                            };
                            phases.insert(
                                FnKey::method(
                                    locus.clone(),
                                    name.clone(),
                                ),
                                PhaseEntry { phase: name, hook },
                            );
                        }
                    }
                    TopDecl::Fn(f) => {
                        decls.insert(
                            f.name.name.clone(),
                            DeclInfo {
                                origin: origin(&f.name.name),
                                span: f.name.span,
                            },
                        );
                    }
                    TopDecl::Type(t) => {
                        decls.insert(
                            t.name.name.clone(),
                            DeclInfo {
                                origin: origin(&t.name.name),
                                span: t.name.span,
                            },
                        );
                    }
                    TopDecl::Interface(i) => {
                        decls.insert(
                            i.name.name.clone(),
                            DeclInfo {
                                origin: origin(&i.name.name),
                                span: i.name.span,
                            },
                        );
                    }
                    TopDecl::Group(g) => {
                        decls.insert(
                            g.name.name.clone(),
                            DeclInfo {
                                origin: origin(&g.name.name),
                                span: g.name.span,
                            },
                        );
                    }
                    TopDecl::Module(md) => {
                        walk(&md.items, seed_of, decls, phases)
                    }
                    _ => {}
                }
            }
        }
        for p in programs {
            walk(&p.items, &seed_of, &mut decls, &mut phases);
        }
        Model { decls, phases, seeds }
    }

    /// The decl a graph node belongs to: its locus for methods,
    /// itself for free fns.
    fn owning_decl<'k>(k: &'k FnKey) -> &'k str {
        k.locus.as_deref().unwrap_or(&k.fn_name)
    }

    /// Is this fn's decl part of the checked bundle (main or an
    /// imported seed)? False for stdlib bodies and synthesized
    /// fns — their spans live in a DIFFERENT offset space, so a
    /// diagnostic must never point at them as if they were bundle
    /// source.
    pub fn is_bundle_fn(&self, k: &FnKey) -> bool {
        self.decls.contains_key(Self::owning_decl(k))
    }

    /// The decl span for a bundle decl name.
    pub fn decl_span(&self, name: &str) -> Option<Span> {
        self.decls.get(name).map(|d| d.span)
    }
}
