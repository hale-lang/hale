//! The check's and the model's test entries, through the snapshot
//! (F.40 phase 4, T3).
//!
//! `hale_types` exported entry points that checked or modelled a bundle
//! no snapshot holds and built the top scope for themselves
//! (`check_program`, `check_bundle`, `check_bundle_opts`,
//! `check_bundle_opts_whole_program`, `check_bundle_opts_scoped`,
//! `check_bundle_for_build`, `derive_application_model`, `effect_certificates`, `resolve_program`,
//! and the bundle forms of `claim_law_diags`, `model_shape_hash` and
//! `dump_topology`). No verb reached them: every verb and the editor
//! build a `hale_frontend::snapshot::Snapshot` and demand the scope, the
//! check and the model from it, and once every test had moved here the
//! entries left `src`. This module offers each of them
//! under the same name, with the same arguments and result, answered by
//! a snapshot, so that a test moved to it by its imports alone. A test
//! file includes it as the codegen tests include their support:
//!
//! ```text
//! #[path = "support/entries.rs"]
//! mod entries;
//! use entries::check_program;
//! ```
//!
//! What each helper builds, and why it is the old entry's equivalent:
//!
//! - [`check_program`]: `Snapshot::from_program` of the program under
//!   `Config::check(true, false)`, then `demand_check`'s diagnostics.
//!   The old entry ran the desugar sequence on a copy, minted it, and
//!   checked the one-program bundle with the whole-program rules on
//!   (`check_bundle_opts_whole_program(.., false)`); the snapshot's load
//!   runs the same sequence and mint, `whole_program: true` turns both
//!   whole-program rules on (strict callees and strict identifiers), and
//!   `allow_unowned_subscriber: false` is the old `false`. The check is
//!   the typing stage followed by the laws stage, which the `demand`
//!   family states is one pass's set over both.
//! - [`check_bundle`] and [`check_bundle_opts`]: the snapshot of the
//!   bundle (below) under `Config::check(false, allow_unowned_subscriber)`:
//!   the partial-program entry, both whole-program rules off, as
//!   `check_bundle_opts_scoped(.., false, false)` had them.
//! - [`check_bundle_opts_whole_program`]: the same under
//!   `Config::check(true, allow_unowned_subscriber)`.
//! - [`check_bundle_for_build`]: that check, then the borrow rule
//!   (`hale_types::build_rule_diags`) over the snapshot's ownership rows,
//!   appended after it as the old entry appended it. (A build's own
//!   snapshot, `Config::build`, runs the rule inside its typing stage,
//!   so its laws' diagnostics follow the rule's; the old order is kept.)
//! - [`check_bundle_opts_scoped`]: `Config::check(strict, ..)` where the
//!   two strictnesses agree. The snapshot holds one whole-program flag,
//!   so a caller that asks for one rule without the other is refused by
//!   a panic that says so.
//! - [`derive_application_model`]: the model's producer
//!   (`model_builder::derive_application_model_over`, `demand_model`'s)
//!   over the families `demand_model` demands from the snapshot of the
//!   bundle (the scope, the bus and ownership graphs, the handler, effect,
//!   form and binding rows, the placement table), under the snapshot's
//!   environment. Two things differ from `demand_model`, both because the
//!   old entry had them: no gate on the typing (the old entry modelled a
//!   program without checking it, and `demand_model` is blocked for a
//!   program that does not typecheck), and the caller's source table
//!   (`Bundle::sources`), when it declared one, on the view the model is
//!   derived over — a bare program's snapshot has no source map, and the
//!   model's provenance is source-backed only through that table. The
//!   configuration is `Config::check(true, false)`; no field of it but the
//!   target reaches a family the model reads.
//! - [`effect_certificates`]: `demand_effect_certificates` of the
//!   snapshot of the bundle under `Config::check(true, false)`: the report
//!   the snapshot's typing produced, which the old entry recomputed over
//!   the same form rows and the entry row's root.
//! - [`claim_law_diags`], [`dump_topology`] (and its alias
//!   [`dump_topology_parts`]), [`model_shape_hash`]: the `*_over` forms
//!   (`judgment::claim_law_diags_over`, `topology::dump_topology_over`,
//!   `topology_projection::project_shape_hash`) over that model, the
//!   snapshot's certificate report and its allocation summary, with the
//!   snapshot's bundle view (the caller's source table on it, as above).
//!   `claim_law_diags` keeps the old entry's gate: no claim surface, no
//!   model, no diagnostics.
//! - [`resolve_program`]: `demand_lowering` of
//!   `Snapshot::from_program(program, import_renames,
//!   Config::harness(host))` with the api and roles asked for, cloned
//!   out of the snapshot. `Config::harness` does not gate lowering on the
//!   check, as the old entry did not. The form, binding, placement and
//!   typed-body rows and the source table the old entry took as
//!   arguments are not read: the snapshot derives its own from the
//!   program (the callers passed empty rows, standing in for the ones
//!   the snapshot now has). A caller that hands the old entry a source
//!   table names its file, and a bare program's snapshot has no file to
//!   name: [`resolve_files`] loads the caller's text under the caller's
//!   name instead, so the view's snapshot seeds each site by its file.
//! - [`check_files`]: what `hale check <dir>` reports for in-memory
//!   files, the api binding included: the load's sequence generates the
//!   binding a program's own `api:` entry asks for, and the check reads
//!   the surface it generated, so a test that holds an api program's
//!   text checks it as the verb does. (A program the caller ran the
//!   api pass over itself has its binding already, so the sequence
//!   generates none and records no surface: [`check_program`] over it
//!   checks the binding without the entry's own rules.)
//!
//! The snapshot of a bundle (`bundle_snapshot`): the bundle's target and
//! import renames on the config and the load, and its program. A bundle
//! of ONE program is that program's snapshot (`Snapshot::from_program`).
//! A bundle of SEVERAL is not a loaded seed, and cannot be made one from
//! the bundle alone: a load reads text, parses each file at its own base
//! of one offset space and through one effect-class table (#345), and
//! merges the files; a hand-built bundle holds programs parsed apart,
//! each from offset 0 with its own effect-class numbering, and keeps
//! them apart. Its programs are merged here as a directory load merges
//! them (`frontend::merge_programs`) and the merge is the snapshot's
//! program: spans keep the overlap the hand-built bundle had, and two
//! files that each declare an effect class number it as their own parses
//! did. A test that holds the files' text loads them as a seed instead:
//! [`load_files`] (in-memory buffers, `Snapshot::load` over an
//! `Overlay`) or [`load_dir`] (a seed directory on disk).

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hale_frontend::frontend::LoadMode;
use hale_frontend::snapshot::{Config, Snapshot, Target};
use hale_frontend::source::{Disk, Overlay};
use hale_model::ApplicationModel;
use hale_syntax::ast::Program;
use hale_syntax::Diag;
use hale_types::effects::EffectCertificates;
use hale_types::resolved::LoweringView;
use hale_types::symbol::SourceFile;
use hale_types::Bundle;

/// Where [`load_files`] puts a test's files: a directory nothing creates.
/// Each file is an overlay buffer under it and nothing is written there.
pub const SEED_DIR: &str = "/hale-test-seed";

/// `hale_types::check_program`, through the snapshot.
pub fn check_program(program: &Program) -> Vec<Diag> {
    checked(&from_program(program.clone(), Vec::new(), Config::check(true, false)))
}

/// `hale_types::check_bundle`, through the snapshot.
pub fn check_bundle(bundle: &Bundle<'_>) -> Vec<Diag> {
    check_bundle_opts(bundle, false)
}

/// `hale_types::check_bundle_opts`, through the snapshot.
pub fn check_bundle_opts(bundle: &Bundle<'_>, allow_unowned_subscriber: bool) -> Vec<Diag> {
    checked(&bundle_snapshot(bundle, Config::check(false, allow_unowned_subscriber)))
}

/// `hale_types::check_bundle_opts_whole_program`, through the snapshot.
pub fn check_bundle_opts_whole_program(bundle: &Bundle<'_>, allow_unowned_subscriber: bool) -> Vec<Diag> {
    checked(&bundle_snapshot(bundle, Config::check(true, allow_unowned_subscriber)))
}

/// `hale_types::check_bundle_opts_scoped`, through the snapshot: one
/// whole-program flag holds both rules.
pub fn check_bundle_opts_scoped(
    bundle: &Bundle<'_>,
    allow_unowned_subscriber: bool,
    strict_callees: bool,
    strict_idents: bool,
) -> Vec<Diag> {
    assert_eq!(
        strict_callees, strict_idents,
        "the snapshot's `Config::whole_program` holds the callee rule and the identifier rule together"
    );
    checked(&bundle_snapshot(bundle, Config::check(strict_callees, allow_unowned_subscriber)))
}

/// `hale_types::check_bundle_for_build`, through the snapshot: the
/// whole-program check, then the rules a build refuses beside it
/// (`hale_types::build_rule_diags`, the borrow rule) over the snapshot's
/// ownership rows, after the check as the old entry appended them.
pub fn check_bundle_for_build(bundle: &Bundle<'_>, allow_unowned_subscriber: bool) -> Vec<Diag> {
    let snap = bundle_snapshot(bundle, Config::check(true, allow_unowned_subscriber));
    let mut diags = checked(&snap);
    let ownership = match snap.demand_ownership_graph() {
        Ok(graph) => &graph.rows,
        Err(blocked) => panic!("the ownership graph is blocked: {}", render_blocked(blocked)),
    };
    diags.extend(hale_types::build_rule_diags(&snap.bundle(), ownership));
    diags
}

/// `hale_types::derive_application_model`, through the snapshot.
pub fn derive_application_model(bundle: &Bundle<'_>) -> ApplicationModel {
    model_of(&bundle_snapshot(bundle, Config::check(true, false)), &bundle.sources)
}

/// `hale_types::effects::effect_certificates`, through the snapshot.
pub fn effect_certificates(bundle: &Bundle<'_>) -> EffectCertificates {
    certificates(&bundle_snapshot(bundle, Config::check(true, false))).clone()
}

/// `hale_types::judgment::claim_law_diags`, through the snapshot.
pub fn claim_law_diags(bundle: &Bundle<'_>) -> Vec<Diag> {
    let snap = bundle_snapshot(bundle, Config::check(true, false));
    let view = view_of(&snap, &bundle.sources);
    if !hale_types::judgment::has_claim_surface(&view) {
        return Vec::new();
    }
    let model = model_of(&snap, &bundle.sources);
    hale_types::judgment::claim_law_diags_over(&view, &model, certificates(&snap), summary(&snap), laws(&snap))
}

/// `hale_types::topology::model_shape_hash`, through the snapshot.
pub fn model_shape_hash(bundle: &Bundle<'_>) -> u64 {
    hale_types::topology_projection::project_shape_hash(&derive_application_model(bundle))
}

/// `hale_types::topology::dump_topology`, through the snapshot.
pub fn dump_topology(bundle: &Bundle<'_>) -> String {
    let snap = bundle_snapshot(bundle, Config::check(true, false));
    let model = model_of(&snap, &bundle.sources);
    let view = view_of(&snap, &bundle.sources);
    hale_types::topology::dump_topology_over(&view, &model, certificates(&snap), summary(&snap), laws(&snap))
}

/// `hale_types::topology::dump_topology_parts`, through the snapshot.
pub fn dump_topology_parts(bundle: &Bundle<'_>) -> String {
    dump_topology(bundle)
}

/// `hale_types::resolved::resolve_program`, through the snapshot. The
/// rows and the source table are the snapshot's own, so the arguments
/// that carried them are not read.
#[allow(clippy::too_many_arguments)]
pub fn resolve_program(
    program: &Program,
    _sources: &[SourceFile],
    import_renames: &[(Vec<String>, String)],
    api: Option<&str>,
    api_roles: Option<&str>,
    _forms: &hale_types::form_rows::FormRows,
    _bindings: &hale_types::binding_rows::BindingRows,
    _placement: &hale_types::placement::PlacementTable,
    _typed: &hale_types::typed_bodies::TypedBodies,
) -> Result<LoweringView, String> {
    let mut config = Config::harness(Target::host());
    config.api = api.map(str::to_string);
    config.api_roles = api_roles.map(str::to_string);
    let snap = from_program(program.clone(), import_renames.to_vec(), config);
    match snap.demand_lowering() {
        Ok(view) => Ok(view.clone()),
        Err(blocked) => Err(render_blocked(blocked)),
    }
}

/// `hale_types::resolved::resolve_program` for a caller that names its
/// files: the seed of `files` loaded as [`load_files`] loads it, under
/// `Config::harness(host)`, and its `demand_lowering` cloned out. The
/// old entry seeded the view's sites by the caller's source table; a
/// load has the table of the files it read, so each user site is seeded
/// by the file it is in, under the caller's name for it.
pub fn resolve_files(files: &[(&str, &str)]) -> Result<LoweringView, String> {
    let snap = load_files(files, Config::harness(Target::host()));
    match snap.demand_lowering() {
        Ok(view) => Ok(view.clone()),
        Err(blocked) => Err(render_blocked(blocked)),
    }
}

/// What `hale check <dir>` reports for a seed of in-memory files: the
/// directory of `files` loaded whole under `Config::check(true, false)`
/// (a directory's check holds the whole-program rules, as
/// `check_program` did), the desugar sequence generating the api binding
/// the program's own `api:` entry asks for, and the load's diagnostics
/// when the load fails, the check's otherwise (`load_for_check`, then
/// `demand_check`). The verb prints its default advisories after these
/// (the unbounded-allocation survey and the borrow rule); they are
/// passes of their own, not the check, and are not here.
pub fn check_files(files: &[(&str, &str)]) -> Vec<Diag> {
    let dir = PathBuf::from(SEED_DIR);
    let buffers = buffers(files);
    match Snapshot::load(&dir, LoadMode::WholeSeed, &Overlay::new(&buffers), Config::check(true, false)) {
        Ok(snap) => checked(&snap),
        Err(hale_frontend::snapshot::LoadError::Load(failure)) => failure.diags,
        Err(hale_frontend::snapshot::LoadError::Refused(msg)) => {
            panic!("`hale check` refuses only an `--api` or `--env` it was not asked for: {msg}")
        }
    }
}

/// A seed of in-memory files, loaded as `hale check <dir>` loads one:
/// each `(name, text)` an overlay buffer at `SEED_DIR/name`, the
/// directory loaded whole (`LoadMode::WholeSeed`) and shaped as `config`
/// says. One file is a seed too.
pub fn load_files(files: &[(&str, &str)], config: Config) -> Snapshot {
    let dir = PathBuf::from(SEED_DIR);
    let buffers = buffers(files);
    let entry = match files {
        [(name, _)] => dir.join(name),
        _ => dir,
    };
    match Snapshot::load(&entry, LoadMode::WholeSeed, &Overlay::new(&buffers), config) {
        Ok(s) => s,
        Err(_) => panic!("the test's seed at {} does not load", entry.display()),
    }
}

/// Each of `files` an overlay buffer under [`SEED_DIR`].
fn buffers(files: &[(&str, &str)]) -> BTreeMap<PathBuf, String> {
    let dir = PathBuf::from(SEED_DIR);
    files.iter().map(|(name, text)| (dir.join(name), text.to_string())).collect()
}

/// A seed directory on disk, loaded whole as `hale check <dir>` loads it.
pub fn load_dir(dir: &Path, config: Config) -> Snapshot {
    match Snapshot::load(dir, LoadMode::WholeSeed, &Disk, config) {
        Ok(s) => s,
        Err(_) => panic!("the seed at {} does not load", dir.display()),
    }
}

/// What a snapshot's check reports: `demand_check`'s diagnostics, in its
/// order.
pub fn checked(snap: &Snapshot) -> Vec<Diag> {
    match snap.demand_check() {
        Ok(c) => c.diags.clone(),
        Err(blocked) => panic!("the check is blocked: {}", render_blocked(blocked)),
    }
}

/// The model of a snapshot, as [`derive_application_model`] derives it:
/// `demand_model`'s producer over `demand_model`'s families, ungated,
/// over the snapshot's view with `sources` on it when the snapshot has
/// no source map of its own.
pub fn model_of(snap: &Snapshot, sources: &[SourceFile]) -> ApplicationModel {
    let blocked = |b: &hale_frontend::snapshot::Blocked| -> ! { panic!("the model's inputs are blocked: {}", render_blocked(b)) };
    let view = view_of(snap, sources);
    let inputs = hale_types::model_builder::ModelInputs {
        top: snap.demand_scope().unwrap_or_else(|b| blocked(b)),
        bus_graph: snap.demand_bus_graph().unwrap_or_else(|b| blocked(b)),
        ownership: snap.demand_ownership_graph().unwrap_or_else(|b| blocked(b)),
        handlers: snap.demand_handlers().unwrap_or_else(|b| blocked(b)),
        effects: snap.demand_effects().unwrap_or_else(|b| blocked(b)),
        forms: snap.demand_forms().unwrap_or_else(|b| blocked(b)),
        bindings: snap.demand_bindings().unwrap_or_else(|b| blocked(b)),
        placement: snap.demand_placement().unwrap_or_else(|b| blocked(b)),
        arrangement: snap.demand_arrangement().unwrap_or_else(|b| blocked(b)),
        dispatch_plan: snap.demand_dispatch_plan().unwrap_or_else(|b| blocked(b)),
        surfaces: snap.demand_surface_rows().unwrap_or_else(|b| blocked(b)),
    };
    hale_types::model_builder::derive_application_model_over(&view, &inputs)
}

fn from_program(program: Program, import_renames: Vec<(Vec<String>, String)>, config: Config) -> Snapshot {
    match Snapshot::from_program(program, import_renames, config) {
        Ok(s) => s,
        Err(_) => panic!("a bare program's snapshot is refused only for an `--api` or `--env` it does not ask for"),
    }
}

/// The snapshot of a bundle a test built (the module header says how a
/// bundle of several programs is held).
fn bundle_snapshot(bundle: &Bundle<'_>, mut config: Config) -> Snapshot {
    config.target = bundle.target.clone();
    let program = match bundle.programs.len() {
        1 => (*bundle.programs.values().next().expect("one program")).clone(),
        _ => hale_frontend::frontend::merge_programs(bundle.programs.values().copied())
            .expect("a bundle with no program has nothing to check"),
    };
    from_program(program, bundle.import_renames.clone(), config)
}

/// The snapshot's bundle view, with `sources` as its source table when
/// the snapshot has none (a bare program's).
fn view_of<'s>(snap: &'s Snapshot, sources: &[SourceFile]) -> Bundle<'s> {
    let mut view = snap.bundle();
    if view.sources.is_empty() {
        view.sources = sources.to_vec();
    }
    view
}

fn certificates(snap: &Snapshot) -> &EffectCertificates {
    match snap.demand_effect_certificates() {
        Ok(c) => c,
        Err(blocked) => panic!("the certificate report is blocked: {}", render_blocked(blocked)),
    }
}

/// The snapshot's law selection (F.40 phase 4, A2): the clauses, the
/// adoption and the environment label the judgment and the artifact read.
fn laws(snap: &Snapshot) -> &hale_types::claims::LawSelection {
    match snap.demand_law_selection() {
        Ok(l) => l,
        Err(blocked) => panic!("law selection is blocked: {}", render_blocked(blocked)),
    }
}

fn summary(snap: &Snapshot) -> &hale_types::alloc_summary::AllocSummary {
    match snap.demand_alloc_summary() {
        Ok(s) => s,
        Err(blocked) => panic!("the allocation summary is blocked: {}", render_blocked(blocked)),
    }
}

fn render_blocked(blocked: &hale_frontend::snapshot::Blocked) -> String {
    let mut out = format!("`{}` is blocked", blocked.family);
    for d in &blocked.because {
        out.push_str(&format!("\n  {}", d.message));
    }
    if let Some(refused) = &blocked.refused {
        out.push_str(&format!("\n  {refused}"));
    }
    out
}
