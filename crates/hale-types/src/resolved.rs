//! The lowering view (F.40 phase 1.2a-ii, a demanded family since
//! phase 2.2b): what codegen lowers.
//!
//! [`resolve_program`] takes the program a verb checked and produces
//! what lowering walks — the user program after the two lowering
//! rewrites, the same program merged with the bundled stdlib —
//! together with the snapshot minted over the merged program and the
//! ownership tables the F.39 pre-pass derives from it. Codegen reads
//! the view; it no longer builds any of it.
//!
//! The view is a family of the frontend's snapshot
//! (`hale_frontend::snapshot::Snapshot::demand_lowering`), which runs
//! this function once, after the check it is gated on, over the
//! snapshot's programs, source map, renames and config. The only other
//! caller is codegen's harness adapter, for a bare program.
//!
//! The two lowering rewrites are the intra-locus rewrite (a publish to
//! a subscriber in the same tree becomes a direct call) and the topic
//! rewrite (every topic reference becomes its wire literal). They are
//! not desugars: each erases a written declaration reference that the
//! checker's laws and the model read, so each runs on lowering's copy
//! of the program, never the checked one, and each is kept as a
//! relation — `intra_locus` and `topic_rewrites`, recorded on the bus
//! graph's subjects — so the program's account still holds what the
//! text no longer does (F.40 phase 2.1b). The intra-locus rewrite is
//! its own stage ([`rewrite_intra_locus`], the snapshot's `intra_locus`
//! family), which the check demands too: rule 10 reads its relation to
//! tell a cycle lowering makes a direct call from one the queue carries
//! (F.40 phase 3, C4).
//!
//! The envelope also carries the ownership graph over the merged
//! program (which accepting ancestor owns each method-body birth, and
//! which locus accepts which child type) and the bubble plans lowering
//! projects from it (F.40 phase 1.3), built here once, the
//! `on_failure` handler rows (F.40 phase 1.4), and the bus graph over
//! the same program with the dispatch plan lowering reads (F.40
//! phase 1.5).
//!
//! After the rewrites, the stdlib is appended and the merged program
//! minted before the pre-pass, with the bundle's source map, so every
//! stdlib and rewrite-generated node has its identity (the mint keeps
//! the ids the bundle already carries and continues the counter) and
//! every site its seed: a user site the file its span falls in, a
//! stdlib site the stdlib's own seed. The pre-pass numbers nothing; a
//! `Struct` or `Call` it finds unnumbered is an error.
//!
//! Before the intra-locus rewrite the user program is minted once
//! more, so every send the relation records is minted on every path,
//! the harness adapter's included.
//!
//! The passes that shape a declaration are not run here: every caller
//! ran the desugar sequence
//! ([`crate::desugar_sequence::desugar_before_check`]) before its
//! check, and this step does not run any of it again (F.40 phase 2.1b).

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{Program, TopDecl};

use hale_model::dispatch_plan::DispatchPlan;
use hale_syntax::desugar::{IntraLocusRewrite, TopicRewrite};

use crate::bus_graph::BusGraph;
use crate::handler_routing::HandlerRouting;
use crate::ownership::OwnerTable;
use crate::ownership_graph::{BubblePlans, OwnershipGraph};
use crate::resolve::TopScope;
use crate::snapshot::Snapshot;
use crate::symbol::{Bundle, SourceFile};

/// The program codegen lowers, and the tables the frontend derives over
/// it: the snapshot's `lowering_view` family.
pub struct LoweringView {
    /// The user's program after the codegen-shape desugars, with the
    /// bundled stdlib's declarations appended: what lowering walks.
    pub merged: Program,
    /// Every site's identity, minted over `merged` after the desugars
    /// with the bundle's source map (ids the bundle already minted are
    /// kept; the counter continues). The stdlib's sites are seeded
    /// under [`crate::snapshot::STDLIB_SEED`].
    pub snapshot: Snapshot,
    pub owner_table: OwnerTable,
    /// Fresh factories, extended by the carrier-return fold.
    pub fresh_locus_factories: BTreeMap<String, (String, Option<String>)>,
    /// Which accepting ancestor owns each method-body birth, and which
    /// locus accepts which child type, over `merged`.
    pub ownership: OwnershipGraph,
    /// The bubble plans lowering acts on, projected from `ownership`.
    pub bubble: BubblePlans,
    /// Which `on_failure` handler a failing child reaches, one row per
    /// handler of `merged` (F.40 phase 1.4).
    pub handlers: HandlerRouting,
    /// Which children are flows, over `merged`: every `release(c: T)`
    /// clause with the locus `T` denotes as lowering names it, and a
    /// generic owner's clause as its template, which lowering
    /// specializes with its own substitution.
    pub flows: crate::flows::FlowRows,
    /// The form rows (F.40 phase 3, C1): the snapshot's, one per user
    /// `@form` declaration with the discipline it gets, inference's
    /// included, and the merged stdlib's as written. Lowering lays each
    /// map out by its row's effective discipline.
    pub forms: crate::form_rows::FormRows,
    /// The binding rows (F.40 phase 3, P2): one per `bindings { }`
    /// entry, the snapshot's. Lowering's prelude reads the entry's
    /// transport, role, adapter, codec and producer-versus-attach here,
    /// and the bus graph's bound-topic set is their projection.
    pub bindings: crate::binding_rows::BindingRows,
    /// The placement table (F.40 phase 3, P1): the snapshot's, where each
    /// instance the deployed program builds runs. Lowering asks it
    /// for its deployment plan and whether any thread crosses the bus
    /// boundary (E2). A user site names the same node index in `merged`.
    pub placement: crate::placement::PlacementTable,
    /// The typed-body table (F.40 phase 3, E4): the snapshot's, what the
    /// checker typed over the program this view lowers, keyed by the
    /// identities the merge kept. Lowering reads the checker's answers
    /// here instead of typing again; the merged stdlib, which the check
    /// does not walk, has no rows.
    pub typed: crate::typed_bodies::TypedBodies,
    /// Whether the program can ever have a bus cell in flight, so
    /// lowering can elide every drain (`crate::bus_inert`).
    pub bus_inert: bool,
    /// The message graph over `merged`, keyed by wire subject (the
    /// topic rewrite has run), with its devirtualization gates (F.40
    /// phase 1.5), and on each subject the sends `intra_locus` rewrote
    /// and the topic references `topic_rewrites` turned into it.
    pub bus: BusGraph,
    /// Lowering's dispatch plan, derived from `bus`'s gates with an
    /// empty domain map: the flavor each subject is lowered to.
    pub plan: DispatchPlan,
    /// Every send the intra-locus rewrite replaced with a direct call:
    /// the relation that keeps the publish in the program's account
    /// after the rewrite erased it from the text (boundary 7).
    pub intra_locus: Vec<IntraLocusRewrite>,
    /// Every topic reference the topic rewrite replaced with its wire
    /// subject: the relation that keeps the declaration a subscribe,
    /// publish or send named in the program's account after the rewrite
    /// erased the name from the text (F.40 phase 2.1b).
    pub topic_rewrites: Vec<TopicRewrite>,
    /// What producing the envelope cost, so a build's phase timing
    /// (`HALE_TIME`, `BuildOptions::time_phases`) can report the
    /// resolve step beside the phases codegen times itself.
    pub resolved_in: std::time::Duration,
    /// The inputs the program was resolved with: the cross-seed rename
    /// table, the `--api` path and the roles the environment binds.
    /// Lowering reads the renames from here, and refuses options whose
    /// api disagrees with the envelope's.
    pub import_renames: Vec<(Vec<String>, String)>,
    pub api: Option<String>,
    pub api_roles: Option<String>,
    /// The top-level scope over `merged`, the one the ownership and bus
    /// graphs were built with.
    pub top: TopScope,
    /// Where lowering routes an allocation, over `merged`: which free
    /// fns are scratch-local (`crate::alloc_routing`).
    pub alloc_routing: crate::alloc_routing::AllocRouting,
    /// The pinned anchors whose nested tree holds a subscriber, by the
    /// name lowering declares them under, from the snapshot's placement
    /// table ([`route_anchors`]): the anchor's mailbox is their route
    /// (the placement correspondence's U-6).
    pub route_anchors: BTreeSet<String>,
    /// The effective target's column of the capability matrix: every
    /// behaviour and obligation lowering emits or omits per target is
    /// read here, and lowering refuses options that name a target of
    /// another class.
    pub cells: crate::capability::LoweringCells,
}

/// The pinned anchors of `table` (a root field placed `pinned`, one per
/// replica, or an adapter in the root's `bindings { }`) with a row
/// below them, in their own template, that realizes a locus `merged`
/// declares a `subscribe` in: each by its realized declaration's
/// lowered name. A locus nested under an anchor runs on the anchor's
/// thread, so its subscriptions need a route there before they
/// register; lowering gives these anchors a mailbox for that, whether or
/// not they subscribe themselves.
pub fn route_anchors(table: &crate::placement::PlacementTable, merged: &Program) -> BTreeSet<String> {
    use crate::placement::DomainKind;
    use hale_syntax::ast::{flat_decls, BusMember, LocusMember};
    let subscribers: BTreeSet<&str> = flat_decls(&merged.items)
        .filter_map(|d| match d {
            TopDecl::Locus(l)
                if l.members.iter().any(|m| {
                    matches!(m, LocusMember::Bus(b)
                        if b.members.iter().any(|bm| matches!(bm, BusMember::Subscribe { .. })))
                }) =>
            {
                Some(l.name.name.as_str())
            }
            _ => None,
        })
        .collect();
    let mut out = BTreeSet::new();
    for domain in &table.domains {
        let DomainKind::Pinned { anchor, .. } = &domain.kind else { continue };
        let Some(anchor_decl) = table.instances.get(anchor).and_then(|r| r.realizes.as_ref()) else {
            continue;
        };
        let nested_subscriber = table.instances.iter().any(|(key, row)| {
            key.origin == anchor.origin
                && key.replica == anchor.replica
                && key.path.len() > anchor.path.len()
                && key.path.starts_with(&anchor.path)
                && row.realizes.as_ref().is_some_and(|d| subscribers.contains(d.lowered.as_str()))
        });
        if nested_subscriber {
            out.insert(anchor_decl.lowered.clone());
        }
    }
    out
}

/// The name the merged program goes by in its bundle view. Nothing is
/// keyed by it outside the envelope; it is kept as codegen named it.
const MERGED_NAME: &str = "__codegen_merged";

/// The bundle view of a merged program: one program under
/// [`MERGED_NAME`] with the build's import renames, no source map, no
/// snapshot. The renames are what let a graph built over it resolve a
/// qualified imported type (`accept(c: lib::Child)`) to the mangled
/// locus the merge declared; without them the graph's `accepts` row
/// for that locus is empty, and lowering reads it as authoritative.
fn merged_bundle<'a>(merged: &'a Program, import_renames: &[(Vec<String>, String)]) -> Bundle<'a> {
    let mut bundle = Bundle::new(std::iter::once((MERGED_NAME.to_string(), merged)).collect());
    bundle.import_renames = import_renames.to_vec();
    bundle
}

impl LoweringView {
    /// The bundle view of `merged` the view's graphs were built over,
    /// with the view's import renames: lowering reads program-wide
    /// facts through it instead of building its own.
    pub fn bundle(&self) -> Bundle<'_> {
        merged_bundle(&self.merged, &self.import_renames)
    }
}

/// The intra-locus rewrite, run on its own (F.40 phase 3, C4): the user
/// program with its rewritten sends replaced by direct calls, and the
/// relation that records them.
/// The snapshot's `intra_locus` family: the check reads the relation
/// (rule 10 judges a cycle synchronous only where lowering makes it a
/// direct call), and lowering continues from the program
/// ([`resolve_rewritten`]), so the two read one rewrite.
pub struct IntraLocusStage {
    pub program: Program,
    pub intra_locus: Vec<IntraLocusRewrite>,
    /// What the rewrite cost, counted in the view's `resolved_in`.
    pub rewritten_in: std::time::Duration,
}

/// Run the intra-locus rewrite over `program`, the one the verb
/// checked. `placement` is the snapshot's placement table
/// (`Snapshot::demand_placement`), which the rewrite reads for the
/// fields off their owner's thread; a caller with none passes
/// `&PlacementTable::default()`, which runs no field off its owner.
pub fn rewrite_intra_locus(
    program: &Program,
    placement: &crate::placement::PlacementTable,
) -> IntraLocusStage {
    // A7 (G16): `BusSubject::QualifiedTopic(alias::Foo)` — cross-seed
    // topic refs the parser admits — are already the plain
    // single-segment `BusSubject::Topic(Ident(mangled_name))` the
    // imported declaration ends up at: the desugar sequence resolved
    // them before the check (`qualified_subjects`), so the rewrite here
    // and the Topic→Literal pass in [`resolve_rewritten`] read the
    // topic's declared wire subject.
    let t_start = std::time::Instant::now();
    let mut program_owned = program.clone();
    // The intra-locus rewrite moves each send's id onto the call that
    // replaces it and records it in the relation, so the sends have to
    // be minted before it runs: a caller that did not mint (the
    // harness adapter, `build_executable_with_options`) would otherwise
    // get a relation of `NodeId::NONE` sends no call can be joined to.
    // Idempotent: the ids a bundle already minted are kept, so the
    // relation names the sends the check's graph holds, and the mint
    // over the merged program in [`resolve_rewritten`] keeps these and
    // continues. A numbering, not a mint: nothing reads rows of the user
    // program on its own, so no snapshot is made of it.
    crate::snapshot::number([&mut program_owned]);
    // A publish into a field the table runs off its owner's thread stays
    // on the bus (F.31 pool safety).
    let intra_locus = hale_syntax::desugar::desugar_intra_locus_topics(
        &mut program_owned,
        &placement.off_owner_fields(),
    );
    IntraLocusStage { program: program_owned, intra_locus, rewritten_in: t_start.elapsed() }
}

/// Resolve `program` into the view codegen lowers: the intra-locus
/// rewrite ([`rewrite_intra_locus`]), then [`resolve_rewritten`]. The
/// snapshot runs the two halves as its `intra_locus` and
/// `lowering_view` families; this is the bare program's entry, and a
/// bare program is the host's.
/// `placement` is the table the rewrite reads ([`rewrite_intra_locus`]).
pub fn resolve_program(
    program: &Program,
    sources: &[SourceFile],
    import_renames: &[(Vec<String>, String)],
    api: Option<&str>,
    api_roles: Option<&str>,
    forms: &crate::form_rows::FormRows,
    bindings: &crate::binding_rows::BindingRows,
    placement: &crate::placement::PlacementTable,
    typed: &crate::typed_bodies::TypedBodies,
) -> Result<LoweringView, String> {
    let host = crate::capability::TargetClass::of(&crate::target::TargetSpec::host())
        .ok_or_else(|| "the host is a target the capability matrix has no column for".to_string())?;
    resolve_rewritten(
        &rewrite_intra_locus(program, placement),
        sources,
        import_renames,
        api,
        api_roles,
        forms,
        bindings,
        placement,
        typed,
        host,
    )
}

/// Resolve the intra-locus rewrite's program into the view codegen
/// lowers: the producer of the snapshot's `lowering_view` family, run by
/// the snapshot and by [`resolve_program`] and by nothing else.
///
/// The stage's program is the one the verb checked, rewritten: it has
/// been through [`crate::desugar_sequence::desugar_before_check`], and
/// nothing here runs that sequence's passes again.
///
/// `sources` is the bundle's source map, the one its snapshot was
/// minted with: the resolved snapshot seeds each user site by the file
/// its span falls in, as the bundle's does. A caller with no source
/// map passes `&[]`, and the user program is then seed 0 by ordinal.
/// `import_renames` is the per-build path-rename table for cross-seed
/// imports (see `hale_codegen::build_executable_with_options`); `api`
/// and `api_roles` are the build's `--api` path and the roles its
/// environment binds, the ones the sequence shaped the api surface
/// with, recorded on the envelope for lowering to hold its options
/// to. `bindings` is the snapshot's binding rows (`Snapshot::demand_bindings`);
/// a caller with none passes `&BindingRows::default()`, and a program
/// with a `bindings { }` entry then has no row for lowering to read.
/// `placement` is the snapshot's placement table
/// (`Snapshot::demand_placement`); a caller with none passes
/// `&PlacementTable::default()`, and lowering then deploys no main locus
/// or anchor route and sees no thread off main, so such a view is fit
/// only for a program that places nothing.
/// `forms` is the snapshot's form rows (`Snapshot::demand_forms`);
/// a caller with none passes `&FormRows::default()`, and every form then
/// gets its written discipline. `typed` is the snapshot's typed-body
/// table (`Snapshot::demand_typed_bodies`); a caller with none passes
/// `&TypedBodies::default()`, and lowering refuses every site that reads
/// a row. `class` is the effective target's column of the capability
/// matrix, the cells the view hands lowering. The error is the message codegen
/// reports as `CodegenError::Unsupported`: a bundled stdlib that does
/// not parse, or a locus-producing node the mint left unnumbered.
pub fn resolve_rewritten(
    stage: &IntraLocusStage,
    sources: &[SourceFile],
    import_renames: &[(Vec<String>, String)],
    api: Option<&str>,
    api_roles: Option<&str>,
    forms: &crate::form_rows::FormRows,
    bindings: &crate::binding_rows::BindingRows,
    placement: &crate::placement::PlacementTable,
    typed: &crate::typed_bodies::TypedBodies,
    class: crate::capability::TargetClass,
) -> Result<LoweringView, String> {
    let t_start = std::time::Instant::now();
    let mut program_owned = stage.program.clone();
    let intra_locus = stage.intra_locus.clone();
    // The two lowering rewrites. They are not desugars: each erases a
    // written declaration reference (a topic name) that the checker's
    // laws and the model read, so they run on lowering's copy, and
    // each returns what it rewrote as a relation the envelope keeps.
    //
    // The intra-locus optimization ran FIRST, in the stage, while sends
    // still carried the cheap `Expr::Ident(Topic)` shape; it rewrote
    // optimizable Send statements into direct `self.handler(...)`
    // method calls. The topic rewrite now turns every remaining
    // `BusSubject::Topic` and `Foo <- expr` into its literal wire
    // subject, so lowering sees only literal subjects, with no
    // topic-specific branching.
    let topic_rewrites = hale_syntax::desugar::desugar_topics(&mut program_owned);
    // The bus-inert verdict, over the user's program before the stdlib
    // merge: whether a bus cell can ever be in flight (`bus_inert`).
    let bus_inert = crate::bus_inert::bus_inert(&program_owned);

    // m73a: parse the bundled stdlib source and merge its decls
    // into the user program before lowering. Stdlib loci land in
    // `user_loci` alongside user-declared loci with no special
    // casing in the lowering passes; collision with user names is
    // prevented by the `__Std*` mangled prefix on bundled decls.
    // The stdlib has been through the same declaration-shaping passes
    // the user program went through before its check (unit returns,
    // construction aliases, the omitted `run`). That the stdlib's loci
    // carry their empty `run` as the user's do matters: pass A2
    // declares lifecycle methods from whichever declaration of a name
    // it keeps, and a user seed that spells a stdlib locus's name (the
    // stdlib's own seeds, harvested into the corpus) would otherwise
    // carry a `run` its bundled twin lacked, and the body lowering
    // would find no declaration.
    let stdlib_program = crate::desugar_sequence::bundled_stdlib()?.clone();
    let mut merged = program_owned;
    let user_items = merged.items.len();
    // The stdlib's items by span, in merge order: the split below
    // checks they are still the tail.
    let stdlib_spans: Vec<_> = stdlib_program.items.iter().map(TopDecl::span).collect();
    merged.items.extend(stdlib_program.items);

    // F.40 phase 1.2a-ii: every site of the merged program has its
    // identity before the pre-pass runs — the stdlib's, and every node
    // a desugar above generated. The ids the bundle minted are kept
    // and the counter continues past them, so the pre-pass below finds
    // every `Struct` and `Call` already numbered.
    //
    // The user's items seed by the bundle's source map; the stdlib's,
    // whose spans overlap the first file's, by the stdlib's own seed.
    // No pass runs between the merge and the mint, so the stdlib's are
    // still the tail the merge appended, and go back after the mint.
    // Asserted: a pass put there that added, removed or reordered a
    // top-level item would seed user items as the stdlib's, or the
    // reverse.
    assert!(
        merged.items.len() == user_items + stdlib_spans.len()
            && merged.items[user_items..].iter().map(TopDecl::span).eq(stdlib_spans.iter().copied()),
        "the merged program's tail is no longer the stdlib's items: a pass between the merge \
         and the mint added, removed or reordered a top-level item"
    );
    let mut stdlib = Program {
        effect_names: Vec::new(),
        declared_effects: Vec::new(),
        effect_defs: Vec::new(),
        imports: Vec::new(),
        items: merged.items.split_off(user_items),
        span: merged.span,
    };
    let snapshot = crate::snapshot::mint(
        [("program", &mut merged), (crate::snapshot::STDLIB_SEED, &mut stdlib)],
        sources,
    );
    merged.items.append(&mut stdlib.items);

    // GH #921 A2: the ownership pre-pass, over the merged and
    // desugared program. It derives an owner for every locus-producing
    // expression node from syntactic position, keyed by the id the
    // mint above gave it, using the same fresh-factory fixpoint
    // lowering uses — extended to the carrier returns that fixpoint
    // misses.
    //
    // GH #921 A3, commit 1: the extension is no longer table-only.
    // `fresh_factories::collect` classifies the CARRIER
    // node and never its arms, so `return if c { make(1) } else {
    // make2(1) }` left `produce` out of the map and its caller's
    // binding did not own the result — the 105-cell carrier-return
    // family. The pre-pass already flattens `if` / `match` / block
    // tails to decide the same question; folding its answer back into
    // the map lowering reads is what closes the family, and it keeps
    // the two sides of every ownership decision computed once.
    //
    // F.40 phase 1.2c: the rows are the checker's too; the pre-pass
    // reads the fresh half, the locus and the returned binding of each
    // row the escape walk passed.
    let mut fresh_locus_factories: BTreeMap<String, (String, Option<String>)> =
        crate::ownership::fresh_factories(&[&merged], &snapshot, import_renames)
            .into_iter()
            .filter_map(|(f, row)| Some((f, (row.locus, row.fresh?.returned_binding))))
            .collect();
    let mut owner_table = crate::ownership::resolve_owners(
        &merged,
        &snapshot,
        &fresh_locus_factories,
        import_renames,
    )
    .map_err(|e| e.to_string())?;
    // F.40 phase 1.2b: and what each `let` needs to know about its
    // own binding, one row per binding site, keyed by the identity
    // minted above.
    crate::ownership::resolve_binding_facts(&merged, &snapshot, &mut owner_table);
    for (fname, locus) in owner_table.extended_fresh_factories() {
        // A carrier return hands back an ARM's value, so there is no
        // single returned binding to name: `None`, the same as a fn
        // whose every `return` is a literal.
        fresh_locus_factories
            .entry(fname.clone())
            .or_insert_with(|| (locus.clone(), None));
    }

    // F.40 phase 1.3: the ownership graph and the bubble plans, over
    // the same merged and desugared program the bus graph is built
    // from. The bundle's one program keeps the name codegen gave it,
    // so nothing keyed by program name moves; the scope's diagnostics
    // are the checker's to report, not this step's.
    //
    // F.40 phase 1.5: and the bus graph and lowering's dispatch plan,
    // over the same bundle and scope. The topic desugars above have
    // run, so every bus-block subject is its wire literal — the string
    // the register and publish sites see — and the stdlib is merged,
    // so the gates are sound against its wildcard subscribers
    // (`log.**`). A bundle with no entry point is open world: every
    // subject is ineligible, and the plan is all dynamic.
    let (ownership, bubble, bus, plan, top) = {
        let bundle = merged_bundle(&merged, import_renames);
        // The scope's diagnostics are dropped: the checker reported
        // them already, over the program the verb checked.
        let (top, _diags) = crate::resolve::build_top_scope(&bundle);
        let graph = crate::ownership_graph::build_ownership_graph(&bundle, &top, placement);
        let bubble = graph.bubble_plans();
        let mut bus = crate::bus_graph::build_bus_graph(&bundle, &top, bindings, placement);
        // Boundary 7: the sends the intra-locus rewrite replaced are
        // gone from `merged`, but not from the graph. Each is recorded
        // on its subject, which the rewrite named by topic and the
        // graph keys by wire. The publisher's `publish` declaration is
        // still in its bus block, so the subject is always there.
        let wires = crate::topic_identity::topic_wire_subjects(&merged.items);
        for rw in &intra_locus {
            let wire = wires.get(&rw.subject).unwrap_or(&rw.subject);
            let info = bus.subjects.get_mut(wire);
            debug_assert!(info.is_some(), "rewritten send on `{wire}` has no subject in the graph");
            if let Some(info) = info {
                info.direct_sends.push((rw.locus.clone(), rw.handler.clone()));
            }
        }
        // And the topic references the topic rewrite turned into wire
        // literals: each is recorded on the subject it now carries, with
        // the declaration it named, so the graph lowering reads still
        // knows the topic behind every literal. A subject no bus block
        // declares (a send from a free fn) has no row to hold it; the
        // envelope's `topic_rewrites` keeps every one regardless.
        for rw in &topic_rewrites {
            if let Some(info) = bus.subjects.get_mut(&rw.wire) {
                info.written_topics.push((rw.site, rw.written.clone()));
            }
        }
        // The gates are the ones the rewritten program was judged by,
        // as before: the relation is recorded, not yet read.
        //
        // The flavor is a function of the gates alone; the domain map
        // only fills the `same_domain` survey column, and lowering's
        // is empty on purpose (#464's widening is its own optimization
        // with its own bench gate). The model derives its plan with
        // the arrangement's domains.
        let plan = hale_model::dispatch_plan::DispatchPlan::from_gates(
            &bus.dispatch_gates(),
            &BTreeMap::new(),
        );
        (graph, bubble, bus, plan, top)
    };

    // F.40 phase 1.4: the handler rows, over the same merged program,
    // with the child type resolved the way lowering resolves it.
    let handlers = crate::handler_routing::handler_rows(&[&merged], import_renames, &snapshot);
    // The flow rows over the same merged program, each clause's child
    // resolved to the locus lowering names: lowering reads flow-ness here.
    let flows = crate::flows::survey(&[&merged], import_renames);
    // The allocation-routing rows over the same merged program, cross-seed
    // calls resolved through the same renames: lowering reads them.
    let alloc_routing = crate::alloc_routing::derive_alloc_routing(&merged, import_renames);
    // The snapshot's form rows, found by the identities the merge kept,
    // and a written-configuration row for every declaration they do not
    // hold (the stdlib's).
    let forms = forms.clone().extended(crate::form_rows::FormRows::configured(&merged.items));
    // The snapshot's typed-body table, found by the identities the merge
    // kept, and the conformance of every pair the merged stdlib adds.
    let typed = typed.extended(&merged.items, &top);
    // The pinned anchors whose nested subscribers route to their mailbox
    // (U-6), by the names the merged program declares.
    let route_anchors = route_anchors(placement, &merged);

    Ok(LoweringView {
        merged,
        snapshot,
        owner_table,
        fresh_locus_factories,
        ownership,
        bubble,
        handlers,
        flows,
        forms,
        bindings: bindings.clone(),
        placement: placement.clone(),
        typed,
        bus_inert,
        bus,
        plan,
        intra_locus,
        topic_rewrites,
        resolved_in: stage.rewritten_in + t_start.elapsed(),
        import_renames: import_renames.to_vec(),
        api: api.map(str::to_string),
        api_roles: api_roles.map(str::to_string),
        top,
        alloc_routing,
        route_anchors,
        cells: crate::capability::LoweringCells::of(class),
    })
}

/// The declaration a qualified path names: the stdlib's path renames
/// first, then the build's cross-seed `import_renames`.
pub(crate) fn lookup_qualified_path(
    segs: &[&str],
    import_renames: &[(Vec<String>, String)],
) -> Option<String> {
    if let Some(s) = crate::ownership::stdlib_mangled_for_path(segs) {
        return Some(s.to_string());
    }
    let key: Vec<String> = segs.iter().map(|s| s.to_string()).collect();
    import_renames
        .iter()
        .find(|(k, _)| k == &key)
        .map(|(_, v)| v.clone())
}
