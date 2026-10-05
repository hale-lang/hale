//! Hale type checker. Phase 1 milestone 2.
//!
//! Public surface:
//! - [`check::check_bundle_reporting`] — the check, over the scope and
//!   the rows a `hale_frontend::snapshot::Snapshot` demands for it
//!   (`Snapshot::demand_check`, which every verb and the editor read).
//! - [`Bundle`] — the compilation-unit shape the checker takes.
//! - [`ty::Ty`] — resolved-type representation.
//!
//! Milestone-2 cut: literal typing, binary/unary op type
//! compatibility, struct-literal field typing, bus-send
//! subject + payload type matching, closure-assertion type
//! compatibility, `self.field` resolution against enclosing
//! locus's params. Externally-imported names (stdlib paths
//! like `time::sleep`, `println` builtins) resolve to
//! `Ty::Unknown` and pass through.
//!
//! Deferred to milestone 3: contract compatibility (F.8),
//! generic instantiation, k_max compile-time computation,
//! closure cycle existence, full call-site signature checking
//! against built-ins.

pub mod alloc_routing;
pub mod arrangement;
pub mod alloc_summary;
pub mod binding_rows;
pub mod borrow_lifetime;
pub mod bare_fallible;
pub mod builtin_sigs;
pub mod builtin_types;
pub mod budget_check;
pub mod bus_graph;
pub mod bus_inert;
pub mod callgraph;
pub mod capability;
pub mod effect_classes;
pub mod effect_rows;
pub mod effects;
pub mod entry;
pub mod evidence;
pub mod frontier;
pub mod check;
pub mod closure_events;
pub mod correspondence;
pub mod handler_routing;
pub mod lifecycle;
pub mod law;
pub mod lowering_laws;
pub mod claim_lowering;
pub mod claims;
pub mod desugar_sequence;
pub mod model;
pub mod judgment;
pub mod mangle;
pub mod model_builder;
pub mod model_query;
pub mod topic_identity;
pub mod topology;
pub mod topology_diff;
pub mod topology_projection;
pub mod model_graph;
pub mod verdict;
pub mod secret_reveal;
pub mod stdlib_names;
pub mod stdlib_bodies;
pub mod stdlib_surface;
pub mod ownership;
pub mod ownership_graph;
pub mod placement;
pub mod printable;
pub mod purity;
pub mod quantitative;
pub mod resolve;
pub mod resolved;
pub mod roles;
pub mod unit_graph;
mod units;
mod qualified_subjects;
pub mod snapshot;
pub mod resource_budget;
pub mod flows;
mod fn_values;
pub mod sealability;
pub mod sealed_access;
pub mod symbol;
pub mod sync_inference;
pub mod form_rows;
pub mod target;
pub mod typed_bodies;
pub mod ty;
pub mod working_set;

/// m94: subject wildcard matching used by the type checker
/// (publish-side authorization for computed subjects) and
/// mirrored at runtime by the C runtime's `lotus_subject_match`.
/// v0 supports a
/// trailing `**` that matches *zero or more* remaining
/// dot-separated segments — `log.app.**` matches the root
/// `log.app` AND any descendant. Both implementations
/// must agree.
/// Re-exported from `hale-model`, which owns the canonical
/// definition — the model needs it to decide which subjects an
/// unresolved publish can address, and cannot depend on this crate.
/// One Rust implementation, not two.
pub use hale_model::wildcard_match;

use hale_syntax::Diag;

pub use crate::symbol::Bundle;
pub use crate::ty::Ty;

/// Render the per-method allocation summary + call graph (GH #18 item
/// 1): the bundle's own fns and loci, judged over the snapshot's summary
/// (`summary`, with the stdlib's analysis copy and the import renames).
/// Drives `--dump-alloc-summary` and the editor's `hale/allocSummary`.
pub fn dump_alloc_summary(summary: &alloc_summary::AllocSummary) -> String {
    summary.render()
}

/// Render the per-program resource budget — the placement table's
/// threads and worker pools, bus subjects, fd sites (GH #18 item 5, count
/// slice). Drives `--dump-resource-budget`. `table` and `summary` are the
/// bundle's snapshot's (`demand_placement`, `demand_alloc_summary`).
pub fn dump_resource_budget(
    bundle: &Bundle<'_>,
    table: &placement::PlacementTable,
    summary: &alloc_summary::AllocSummary,
) -> String {
    resource_budget::budget_for_programs(bundle, table, summary).render()
}

/// Bound-solver warnings: one per unbounded-accumulation allocation site
/// (GH #18 item 1). `include_all = false` reports only sites inside a
/// `@bounded` locus (the always-on in-source opt-in); `true` is the
/// whole-program survey behind `--warn-unbounded-alloc`. `@unbounded`-fn
/// sites are suppressed in both modes, and so is a site with no author
/// position (`alloc_summary::AuthorPositions`). `summary` is the
/// bundle's snapshot's (`demand_alloc_summary`).
pub fn unbounded_alloc_warnings(
    bundle: &Bundle<'_>,
    summary: &alloc_summary::AllocSummary,
    include_all: bool,
) -> Vec<Diag> {
    let progs: Vec<&hale_syntax::ast::Program> = bundle.programs.values().copied().collect();
    alloc_summary::unbounded_alloc_diags(summary, &progs, &bundle.snapshot, &bundle.sources, include_all)
}

/// Resource-leak warnings: an fd-acquiring call whose result is stored
/// resident in an unbounded context (GH #18 item 5, leak stage). Opt-in
/// via `--warn-resource-leak`. `summary` is the bundle's snapshot's.
pub fn resource_leak_warnings(summary: &alloc_summary::AllocSummary) -> Vec<Diag> {
    resource_budget::resource_leak_diags(summary)
}

/// Check a bundle's resource counts against declared ceilings (GH #18 item
/// 5, the CI gate). Returns one violation message per over-budget resource
/// (empty = within budget). Drives `--check-resource-budget`. `table` and
/// `summary` are the bundle's snapshot's.
pub fn check_resource_ceiling(
    bundle: &Bundle<'_>,
    table: &placement::PlacementTable,
    summary: &alloc_summary::AllocSummary,
    ceiling: &resource_budget::ResourceCeiling,
) -> Vec<String> {
    let budget = resource_budget::budget_for_programs(bundle, table, summary);
    resource_budget::check_ceiling(&budget, ceiling)
}

/// The rules a build refuses beside the check, after it: the borrow
/// rule (GH #730, #1048) — a handle stored by name into a
/// locus-carrying field, or kept by a method (`Router.add`), is never
/// the holder's to reclaim, so it must outlive the holder — an error on
/// every build path, as it is in `hale check`. The snapshot's check appends them for a build's config
/// (`Config::build_rules`). A bare fallible call (GH #738) is the
/// check's own: the `bare_fallible` law runs with the typing.
/// `ownership` is the rows of the bundle's ownership graph
/// (`Snapshot::demand_ownership_graph`): the rule reads which locus
/// accepts which child there (`OwnershipRows::accepts_ancestor`).
pub fn build_rule_diags(bundle: &Bundle<'_>, ownership: &ownership_graph::OwnershipRows) -> Vec<Diag> {
    let programs: Vec<&hale_syntax::ast::Program> = bundle.programs.values().copied().collect();
    let mut diags = borrow_lifetime::borrow_lifetime_diags_with_renames(
        &programs,
        &bundle.snapshot,
        &bundle.import_renames,
        ownership,
    );
    stdlib_bodies::demangle_imports(&mut diags, &[]);
    diags
}

/// A bundle no snapshot holds, with identities: `bundle` itself when an
/// entry already minted it (every verb's snapshot), and
/// otherwise its programs copied and minted once, together, with its
/// source map — what `Bundle::new` over parsed programs hands
/// [`check::check_bundle`]. Every family the check reads is then derived from the one
/// numbered program, as the snapshot's are: the bus graph's sends and
/// the intra-locus rewrite's relation ([`bundle_intra_locus`], whose own
/// numbering keeps these ids) name a send by the same id, so rule 10's
/// join of the two answers (F.40 phase 3, C4).
pub(crate) fn with_identities<R>(bundle: &Bundle<'_>, f: impl FnOnce(&Bundle<'_>) -> R) -> R {
    if !bundle.snapshot.is_empty() {
        return f(bundle);
    }
    let mut programs: Vec<(String, hale_syntax::ast::Program)> =
        bundle.programs.iter().map(|(name, p)| (name.clone(), (*p).clone())).collect();
    let snapshot =
        snapshot::mint(programs.iter_mut().map(|(name, p)| (name.as_str(), p)), &bundle.sources);
    f(&Bundle {
        programs: programs.iter().map(|(name, p)| (name.clone(), p)).collect(),
        import_renames: bundle.import_renames.clone(),
        sources: bundle.sources.clone(),
        target: bundle.target.clone(),
        snapshot,
    })
}

/// Law selection over a bundle no snapshot holds, which names no
/// deployment environment: what [`check::check_bundle`] and
/// [`claim_lowering::lower_claims`] read, once per call. Every verb reads its
/// snapshot's (`Snapshot::demand_law_selection`).
pub fn bundle_law_selection(bundle: &Bundle<'_>) -> claims::LawSelection {
    let programs: Vec<&hale_syntax::ast::Program> = bundle.programs.values().copied().collect();
    claims::select_laws(&programs, &bundle.import_renames, &claims::EnvBinding::default())
}

/// The served surface of a bundle no snapshot holds: the api entry of
/// the entry row's root (`entry`), over the bundle's programs as they
/// stand, which the sequence has already generated the binding into, or
/// not (a test that generated it itself). What [`check::check_bundle`]
/// reads; every
/// verb reads its snapshot's, the surface its sequence generated the
/// binding from (`Snapshot::api_surface`).
pub fn bundle_api_surface(bundle: &Bundle<'_>, entry: &entry::EntryRow) -> Option<hale_syntax::api_gen::ApiSurface> {
    let programs: Vec<&hale_syntax::ast::Program> = bundle.programs.values().copied().collect();
    hale_syntax::api_gen::api_surface(&programs, entry.root().and_then(|m| m.decl(bundle)))
}

/// The handler rows of a bundle no snapshot holds, in the bundle's
/// order (a row's position is its authored ordinal): what
/// [`check::check_bundle`] reads. Every verb
/// reads its snapshot's (`Snapshot::demand_handlers`).
pub(crate) fn bundle_handler_rows(bundle: &Bundle<'_>) -> handler_routing::HandlerRouting {
    let programs: Vec<&hale_syntax::ast::Program> = bundle.programs.values().copied().collect();
    handler_routing::handler_rows(&programs, &bundle.import_renames, &bundle.snapshot)
}

/// The flow rows of a bundle no snapshot holds: what
/// [`check::check_bundle`] reads.
/// Every verb reads its snapshot's (`Snapshot::demand_flows`).
pub(crate) fn bundle_flow_rows(bundle: &Bundle<'_>) -> flows::FlowRows {
    let programs: Vec<&hale_syntax::ast::Program> = bundle.programs.values().copied().collect();
    flows::survey(&programs, &bundle.import_renames)
}

/// The intra-locus rewrite's relation for a bundle no snapshot holds
/// (the test entries), over its programs merged as a build merges them:
/// what the snapshot's `intra_locus` family holds for a verb. The
/// bundle is one [`with_identities`] numbered, so the rewrite's
/// numbering of the merge keeps every send's id and the relation names
/// the sends the bundle's bus graph holds. `placement` is the bundle's
/// table, whose off-owner fields the rewrite keeps on the bus.
pub(crate) fn bundle_intra_locus(
    bundle: &Bundle<'_>,
    placement: &placement::PlacementTable,
) -> Vec<hale_syntax::desugar::IntraLocusRewrite> {
    let mut programs = bundle.programs.values();
    let Some(first) = programs.next() else { return Vec::new() };
    let mut merged = (*first).clone();
    for p in programs {
        merged.items.extend(p.items.iter().cloned());
    }
    resolved::rewrite_intra_locus(&merged, placement).intra_locus
}


/// Whether a program the resolver and the checker reported `diags`
/// for denotes a model: no error but a claim's. Claim errors do not
/// gate it: a program whose only errors are broken LAWS still has a
/// valid model, and refusing to judge the rest of its claims because
/// one of them failed would hide law violations behind each other.
pub fn denotes_a_model(diags: &[Diag]) -> bool {
    !diags.iter().any(|d| d.is_error() && d.kind != hale_syntax::error::DiagKind::Claim)
}

/// The check's last step, over everything it reported: the user's
/// spelling, and no repeated diagnostic.
pub fn finish_check_diags(diags: &mut Vec<Diag>) {
    finish_check_diags_after(&[], diags);
}

/// The same last step over diagnostics that follow `prior`, a list
/// this step already finished: `diags` in the user's spelling, with no
/// diagnostic that repeats one of `prior` or an earlier one of its own.
/// It leaves `diags` exactly the tail [`finish_check_diags`] leaves
/// after `prior` in `prior ++ diags`, so a check finished in two stages
/// (the snapshot's typing, then its laws) reports what one pass over
/// both would.
pub fn finish_check_diags_after(prior: &[Diag], diags: &mut Vec<Diag>) {
    // GH #470: diagnostics speak the user's spelling at EVERY
    // consumer — CLI, LSP, library callers, tests — not just the
    // CLI, which used to be the only layer applying the stdlib
    // demangle. A message naming `__StdHttpMiddleware` points at a
    // symbol that appears nowhere in the author's source. (The
    // CLI's own demangle pass additionally rewrites cross-seed
    // import renames, which only it knows; re-rewriting an
    // already-public stdlib name there is a no-op.)
    stdlib_bodies::demangle_imports(diags, &[]);
    // GH #469 A4: drop exact duplicates — same kind, same span,
    // same message.
    //
    // Several expressions get visited twice (an argument once by
    // the generic call path and again by the bare-builtin argument
    // check, for instance), so a single mistake could be reported
    // twice. That was invisible while interpolation spans all
    // collapsed to `1:1`; fixing the spans made it visible, which
    // is the usual way this kind of thing surfaces.
    //
    // Deduplicating on the whole diagnostic — not on the span — is
    // deliberate: two DIFFERENT problems at one span are both worth
    // saying, and only a byte-identical repeat is noise. Order is
    // preserved so the first report keeps its position.
    let key = |d: &Diag| (format!("{:?}", d.kind), d.span.start.as_usize(), d.span.end.as_usize(), d.message.clone());
    let mut seen: std::collections::HashSet<_> = prior.iter().map(key).collect();
    diags.retain(|d| seen.insert(key(d)));
}

#[cfg(test)]
mod flat_shapeable_tests {
    //! Form K (2026-05-20): `is_flat_shapeable` predicate
    //! drives the route-selection matrix for the bus-decl
    //! constraint substrate. These tests pin the predicate's
    //! behavior on the cases the route matrix consults.

    use std::collections::BTreeMap;

    use hale_syntax::ast::PrimType;
    use hale_syntax::parse_source;

    use crate::resolve::{build_top_scope, TopScope};
    use crate::symbol::Bundle;
    use crate::ty::{is_flat_shapeable, Ty};

    fn with_scope(src: &str, f: impl FnOnce(&TopScope)) {
        let p = parse_source(src).expect("parses");
        let mut programs = BTreeMap::new();
        programs.insert(String::new(), &p);
        let bundle = Bundle::new(programs);
        let (scope, _) = build_top_scope(&bundle);
        f(&scope);
    }

    #[test]
    fn flat_for_pure_primitives() {
        with_scope("fn main() {}", |s| {
            for p in [
                PrimType::Int,
                PrimType::Uint,
                PrimType::Float,
                PrimType::Decimal,
                PrimType::Bool,
                PrimType::Time,
                PrimType::Duration,
            ] {
                assert!(
                    is_flat_shapeable(&Ty::Prim(p), s),
                    "primitive {:?} should be flat",
                    p
                );
            }
        });
    }

    #[test]
    fn not_flat_for_string_and_bytes() {
        with_scope("fn main() {}", |s| {
            for p in [
                PrimType::String,
                PrimType::Bytes,
                PrimType::BytesView,
                PrimType::StringView,
            ] {
                assert!(
                    !is_flat_shapeable(&Ty::Prim(p), s),
                    "variadic primitive {:?} should NOT be flat",
                    p
                );
            }
        });
    }

    #[test]
    fn flat_for_scalar_fixed_size_array() {
        // 2026-07-01 inline fixed arrays: a scalar-element `[T; N]` is
        // laid out INLINE by codegen (array_inline_spec), so its element
        // bytes are part of the value's own layout — memcpy-flat.
        with_scope("fn main() {}", |s| {
            let ty = Ty::Array(Box::new(Ty::Prim(PrimType::Int)), Some(8));
            assert!(is_flat_shapeable(&ty, s));
        });
    }

    #[test]
    fn not_flat_for_unbounded_array() {
        with_scope("fn main() {}", |s| {
            let ty = Ty::Array(Box::new(Ty::Prim(PrimType::Int)), None);
            assert!(!is_flat_shapeable(&ty, s));
        });
    }

    #[test]
    fn not_flat_for_array_of_string() {
        with_scope("fn main() {}", |s| {
            let ty = Ty::Array(Box::new(Ty::Prim(PrimType::String)), Some(4));
            assert!(!is_flat_shapeable(&ty, s));
        });
    }

    #[test]
    fn flat_for_struct_of_primitives() {
        with_scope(
            "type Quote { bid: Decimal; ask: Decimal; venue: Int; } fn main() {}",
            |s| {
                let ty = Ty::Named("Quote".to_string());
                assert!(is_flat_shapeable(&ty, s));
            },
        );
    }

    #[test]
    fn not_flat_for_struct_with_string_field() {
        with_scope(
            "type Note { code: Int; text: String; } fn main() {}",
            |s| {
                let ty = Ty::Named("Note".to_string());
                assert!(!is_flat_shapeable(&ty, s));
            },
        );
    }

    #[test]
    fn flat_for_nested_struct_when_all_flat() {
        with_scope(
            "type Inner { v: Int; } type Outer { a: Inner; b: Decimal; } fn main() {}",
            |s| {
                let ty = Ty::Named("Outer".to_string());
                assert!(is_flat_shapeable(&ty, s));
            },
        );
    }

    #[test]
    fn not_flat_for_unknown_named() {
        with_scope("fn main() {}", |s| {
            let ty = Ty::Named("NoSuchType".to_string());
            // Conservative: predicate cannot assert flatness
            // for a type it cannot see.
            assert!(!is_flat_shapeable(&ty, s));
        });
    }

    #[test]
    fn not_flat_for_fallible() {
        with_scope("fn main() {}", |s| {
            let ty = Ty::Fallible {
                success: Box::new(Ty::Prim(PrimType::Int)),
                payload: Box::new(Ty::Prim(PrimType::Int)),
            };
            assert!(!is_flat_shapeable(&ty, s));
        });
    }

    #[test]
    fn flat_for_unit() {
        with_scope("fn main() {}", |s| {
            assert!(is_flat_shapeable(&Ty::Unit, s));
        });
    }
}

/// GH #265: render the whole-bundle `.hale.effects` manifest —
/// declared contracts plus inferred effect sets, sorted for stable
/// diffs. The CLI writes this next to `.hale.topo`; a diff in review
/// is an effect regression.
pub fn dump_effects_manifest(bundle: &Bundle<'_>, effects: &effect_rows::EffectRows) -> String {
    let programs: Vec<&hale_syntax::ast::Program> =
        bundle.programs.values().copied().collect();
    let mut rows = crate::effects::effect_manifest_with_inference(&programs, effects);
    // An app's manifest describes the APP, not the libraries it
    // imports.
    //
    // Once `check` began resolving imports, every imported fn started
    // emitting its own row: one downstream fleet's committed baseline
    // went from 1,319 rows to 8,021, with 131 of one app's 151 rows
    // being merged symbols. That defeats the artifact's whole purpose
    // — an effect regression is supposed to be a one-line diff in
    // review, and a mangled name is both unreadable and encodes an
    // internal scheme that churns.
    //
    // Dropping them loses nothing. A library's own rows come from
    // checking that library (how a multi-seed project is baselined
    // anyway), and what an imported fn contributes to THIS app is
    // already visible in the caller's inferred `does={…}`.
    rows.retain(|r| !is_merged_symbol(&r.func));
    crate::effects::render_effect_manifest(&rows)
}

/// Merged cross-seed symbols carry the `__lib_` prefix the import
/// resolver mangles them with — on the fn itself or on its locus.
fn is_merged_symbol(name: &str) -> bool {
    name.split("::").any(|seg| seg.starts_with("__lib_"))
}
