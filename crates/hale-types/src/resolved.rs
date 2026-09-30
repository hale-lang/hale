//! The resolved program (F.40 phase 1.2a-ii): the envelope codegen
//! lowers.
//!
//! [`resolve_program`] takes the program a verb checked and produces
//! what lowering walks — the user program after the codegen-shape
//! desugars, the same program merged with the bundled stdlib and
//! normalized — together with the snapshot minted over the merged
//! program and the ownership tables the F.39 pre-pass derives from it.
//! Codegen reads the envelope; it no longer builds any of it.
//!
//! The envelope also carries the ownership graph over the merged
//! program (which accepting ancestor owns each method-body birth, and
//! which locus accepts which child type) and the bubble plans lowering
//! projects from it (F.40 phase 1.3), built here once, the
//! `on_failure` handler rows (F.40 phase 1.4), and the bus graph over
//! the same program with the dispatch plan lowering reads (F.40
//! phase 1.5).
//!
//! The sequence is codegen's former one, moved here unchanged: the
//! same passes, in the same order, over the same inputs. The one
//! addition is the mint over the merged program before the pre-pass,
//! with the bundle's source map, so every stdlib and desugar-generated
//! node has its identity (the mint keeps the ids the bundle already
//! carries and continues the counter) and every site its seed: a user
//! site the file its span falls in, a stdlib site the stdlib's own
//! seed. The pre-pass numbers nothing; a `Struct` or `Call` it finds
//! unnumbered is an error.
//!
//! Today the verbs still run `json_gen`, api injection and sync
//! inference before the check, and this step re-runs the idempotent
//! ones on its own clone, exactly as codegen did. Phase 2 makes this
//! the only place the sequence runs, before the check.

use std::collections::BTreeMap;

use hale_syntax::ast::{LocusMember, Program, TopDecl, TypeExpr};

use hale_model::dispatch_plan::DispatchPlan;
use hale_syntax::desugar::IntraLocusRewrite;

use crate::bus_graph::BusGraph;
use crate::handler_routing::HandlerRouting;
use crate::ownership::OwnerTable;
use crate::ownership_graph::{BubblePlans, OwnershipGraph};
use crate::resolve::TopScope;
use crate::snapshot::Snapshot;
use crate::symbol::{Bundle, SourceFile};

/// The program codegen lowers, and the tables the frontend derives over it.
pub struct ResolvedProgram {
    /// The user's program after the codegen-shape desugars, before the
    /// stdlib merge: a second copy of `merged`'s user half, which only
    /// codegen's tier-1 bus-inert Debug scan reads (a registered legacy
    /// row of the `bus_inert` family, until the verdict is a row of the
    /// envelope).
    pub user: Program,
    /// `user` with the bundled stdlib's declarations appended, unit
    /// returns normalized, construction aliases resolved and the
    /// omitted `run` synthesized: what lowering walks.
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
    /// The message graph over `merged`, keyed by wire subject (the
    /// topic desugars have run), with its devirtualization gates
    /// (F.40 phase 1.5), and on each subject the sends `intra_locus`
    /// rewrote.
    pub bus: BusGraph,
    /// Lowering's dispatch plan, derived from `bus`'s gates with an
    /// empty domain map: the flavor each subject is lowered to.
    pub plan: DispatchPlan,
    /// Every send the intra-locus rewrite replaced with a direct call:
    /// the relation that keeps the publish in the program's account
    /// after the rewrite erased it from the text (boundary 7).
    pub intra_locus: Vec<IntraLocusRewrite>,
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
}

/// The name the merged program goes by in its bundle view. Nothing is
/// keyed by it outside the envelope; it is kept as codegen named it.
const MERGED_NAME: &str = "__codegen_merged";

/// The bundle view of a merged program: one program under
/// [`MERGED_NAME`], no import renames, no source map, no snapshot.
fn merged_bundle(merged: &Program) -> Bundle<'_> {
    Bundle::new(std::iter::once((MERGED_NAME.to_string(), merged)).collect())
}

impl ResolvedProgram {
    /// The bundle view of `merged` the envelope's graphs were built
    /// over: lowering reads program-wide facts through it instead of
    /// building its own.
    pub fn bundle(&self) -> Bundle<'_> {
        merged_bundle(&self.merged)
    }
}

/// Resolve `program` into the envelope codegen lowers.
///
/// `sources` is the bundle's source map, the one its snapshot was
/// minted with: the resolved snapshot seeds each user site by the file
/// its span falls in, as the bundle's does. A caller with no source
/// map passes `&[]`, and the user program is then seed 0 by ordinal.
/// `import_renames` is the per-build path-rename table for cross-seed
/// imports (see `hale_codegen::build_executable_with_options`); `api`
/// and `api_roles` are the build's `--api` path and the roles its
/// environment binds. The error is the message codegen reports as
/// `CodegenError::Unsupported`: a refused `--api` injection, a bundled
/// stdlib that does not parse, or a locus-producing node the mint left
/// unnumbered.
pub fn resolve_program(
    program: &Program,
    sources: &[SourceFile],
    import_renames: &[(Vec<String>, String)],
    api: Option<&str>,
    api_roles: Option<&str>,
) -> Result<ResolvedProgram, String> {
    // A7 (G16): resolve `BusSubject::QualifiedTopic(alias::Foo)`
    // — cross-seed topic refs the parser admits — to plain
    // single-segment `BusSubject::Topic(Ident(mangled_name))`
    // BEFORE desugar runs. The mangling table built by the CLI
    // (`import_renames`) plus the static stdlib path-renames hold
    // every alias-qualified topic decl in the merged program;
    // looking up the path here gives the same mangled name the
    // topic decl ends up at, so desugar's existing Topic→Literal
    // pass uses the topic's declared wire subject. The fallback
    // keeps the leaf segment name so a downstream "unknown topic"
    // diagnostic has something to cite.
    let t_start = std::time::Instant::now();
    let mut program_owned = program.clone();
    resolve_qualified_bus_subjects(&mut program_owned, import_renames);
    // Topic-reference desugaring: rewrite `BusSubject::Topic`
    // and `Foo <- expr` (where Foo is a topic) into the
    // equivalent literal-subject forms. The rest of codegen
    // sees only the legacy AST shape, no topic-specific
    // branching needed.
    //
    // The intra-locus optimization runs FIRST while sends still
    // carry the cheap `Expr::Ident(Topic)` shape; it rewrites
    // optimizable Send statements into direct `self.handler(...)`
    // method calls. desugar_topics then handles whatever bus refs
    // remain.
    // JSON Tier 2: synthesize `__json_parse_<T>` + rewrite `T::from_json`.
    // Idempotent — a no-op if the CLI already generated them pre-typecheck.
    hale_syntax::json_gen::generate_json_parsers(&mut program_owned);
    // GH #1106: the api binding, as ordinary loci and topics. The CLI
    // ran this before the checker; a caller that builds straight from
    // a program (a test) gets it here. Idempotent, and `--api` without
    // an entry in the source injects one first.
    if let Some(path) = api {
        hale_syntax::api_gen::inject_api_entry(&mut program_owned, path)?;
    }
    hale_syntax::api_gen::generate_api(&mut [&mut program_owned], api_roles);
    let intra_locus =
        hale_syntax::desugar::desugar_intra_locus_topics(&mut program_owned);
    hale_syntax::desugar::desugar_topics(&mut program_owned);
    // Proposal A′: rewrite repr-tagged field accessors (`L2::price(v)` /
    // `L2::set_price(w, x)`) into the equivalent `std::bytes::*` calls.
    hale_syntax::desugar::desugar_repr_accessors(&mut program_owned);
    let user = program_owned;

    // m73a: parse the bundled stdlib source and merge its decls
    // into the user program before lowering. Stdlib loci land in
    // `user_loci` alongside user-declared loci with no special
    // casing in the lowering passes; collision with user names is
    // prevented by the `__Std*` mangled prefix on bundled decls.
    let stdlib_program = hale_syntax::parse_source(hale_stdlib::AP_SOURCE)
        .map_err(|diags| {
            let summary = diags
                .iter()
                .map(|d| format!("{:?}", d))
                .collect::<Vec<_>>()
                .join("; ");
            format!("stdlib parse: {}", summary)
        })?;
    let mut merged = user.clone();
    let user_items = merged.items.len();
    // The stdlib's items by span, in merge order: the split below
    // checks they are still the tail.
    let stdlib_spans: Vec<_> = stdlib_program.items.iter().map(TopDecl::span).collect();
    merged.items.extend(stdlib_program.items);
    // Downstream handoff: `-> ()` is a no-op unit annotation. The
    // fallible decl paths already recognized the empty tuple as
    // Unit, but non-fallible methods and every call-site MethodSig
    // consumer hit the 0-element-tuple reject. Normalize ONCE on
    // the merged AST so `-> ()` and "no return type" are the same
    // program everywhere downstream.
    normalize_unit_return_annotations(&mut merged.items);
    // GH #831: and normalize the other spelling nothing downstream
    // should have to know about. `type Row2 = Row;` makes `Row2` a
    // second spelling of `Row` in every TYPE position (GH #759); the
    // CONSTRUCTION positions — `Row2 { }`, `Row2::Variant` — are read
    // at roughly twenty `Expr::Struct` / variant-path sites in the
    // lowering, none of which hold the alias table. Resolving the
    // alias ONCE on the merged AST is what keeps `build` agreeing
    // with `check`, which answers the same question in one hop from
    // its own expanded table.
    crate::mangle::resolve_construction_aliases(&mut merged, import_renames);
    // GH #735: an omitted `run` is an empty `run`, so a flow child is
    // reclaimed when its (empty) run completes on both spellings. On
    // the MERGED program, so a bundled stdlib locus is treated as a
    // user one: pass A2 declares lifecycle methods from whichever
    // declaration of a name it keeps, and a user seed that spells a
    // stdlib locus's name (the stdlib's own seeds, harvested into
    // the corpus) would otherwise carry a `run` its bundled twin
    // lacked, and the body lowering would find no declaration.
    hale_syntax::desugar::desugar_omitted_run(&mut merged);

    // F.40 phase 1.2a-ii: every site of the merged program has its
    // identity before the pre-pass runs — the stdlib's, and every node
    // a desugar above generated. The ids the bundle minted are kept
    // and the counter continues past them, so the pre-pass below finds
    // every `Struct` and `Call` already numbered.
    //
    // The user's items seed by the bundle's source map; the stdlib's,
    // whose spans overlap the first file's, by the stdlib's own seed.
    // No pass since the merge (the unit-return normalization, the
    // construction aliases, the omitted run) adds, removes or reorders
    // a top-level item, so the stdlib's are still the tail the merge
    // appended, and go back after the mint. Asserted: a pass that broke
    // it would seed user items as the stdlib's, or the reverse.
    assert!(
        merged.items.len() == user_items + stdlib_spans.len()
            && merged.items[user_items..].iter().map(TopDecl::span).eq(stdlib_spans.iter().copied()),
        "the merged program's tail is no longer the stdlib's items: a pass between the merge \
         and the mint (normalize_unit_return_annotations, resolve_construction_aliases, \
         desugar_omitted_run) added, removed or reordered a top-level item"
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
    // reads the locus and the returned binding of each.
    let mut fresh_locus_factories: BTreeMap<String, (String, Option<String>)> =
        crate::ownership::fresh_factories(&[&merged], import_renames)
            .into_iter()
            .map(|(f, row)| (f, (row.locus, row.returned_binding)))
            .collect();
    let mut owner_table = crate::ownership::resolve_owners(
        &merged,
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
        let bundle = merged_bundle(&merged);
        // The scope's diagnostics are dropped: the checker reported
        // them already, over the program the verb checked.
        let (top, _diags) = crate::resolve::build_top_scope(&bundle);
        let graph = crate::ownership_graph::build_ownership_graph(&bundle, &top);
        let bubble = graph.bubble_plans();
        let mut bus = crate::bus_graph::build_bus_graph(&bundle, &top);
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

    Ok(ResolvedProgram {
        user,
        merged,
        snapshot,
        owner_table,
        fresh_locus_factories,
        ownership,
        bubble,
        handlers,
        bus,
        plan,
        intra_locus,
        resolved_in: t_start.elapsed(),
        import_renames: import_renames.to_vec(),
        api: api.map(str::to_string),
        api_roles: api_roles.map(str::to_string),
        top,
    })
}

/// `-> ()` is spelled unit: rewrite an empty-tuple return
/// annotation to "no return type" on every fn-shaped declaration,
/// so downstream signature consumers never see a 0-element tuple.
fn normalize_unit_return_annotations(items: &mut [TopDecl]) {
    fn norm(ret: &mut Option<TypeExpr>) {
        if matches!(ret, Some(TypeExpr::Tuple(parts, _)) if parts.is_empty())
        {
            *ret = None;
        }
    }
    for item in items {
        match item {
            TopDecl::Fn(f) => norm(&mut f.ret),
            TopDecl::Interface(i) => {
                for m in &mut i.methods {
                    norm(&mut m.ret);
                }
            }
            TopDecl::Locus(l) => {
                for member in &mut l.members {
                    match member {
                        LocusMember::Fn(f) => norm(&mut f.ret),
                        LocusMember::Mode(md) => norm(&mut md.ret),
                        LocusMember::Lifecycle(lc) => norm(&mut lc.ret),
                        _ => {}
                    }
                }
            }
            TopDecl::Module(m) => {
                normalize_unit_return_annotations(&mut m.items)
            }
            _ => {}
        }
    }
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

/// A7 (G16): walk the program before desugar and resolve every
/// `BusSubject::QualifiedTopic(alias::Foo)` ref to the mangled
/// single-segment ident the imported topic decl ends up at.
/// Leaves the variant in place if the path doesn't resolve so a
/// downstream "unknown topic" diagnostic can cite the source path.
fn resolve_qualified_bus_subjects(
    program: &mut hale_syntax::ast::Program,
    import_renames: &[(Vec<String>, String)],
) {
    use hale_syntax::ast::{
        BusMember, BusSubject, Ident, LocusMember, TopDecl,
    };
    use lookup_qualified_path as lookup;
    fn rewrite(
        subject: &mut BusSubject,
        import_renames: &[(Vec<String>, String)],
    ) {
        if let BusSubject::QualifiedTopic(qn) = subject {
            let segs: Vec<&str> =
                qn.segments.iter().map(|s| s.name.as_str()).collect();
            if let Some(mangled) = lookup(&segs, import_renames) {
                let span = qn.span;
                *subject = BusSubject::Topic(Ident { name: mangled, span });
            }
        }
    }
    // GH #527 B6: `bindings { alias::Topic: unix(...); }` — the
    // entry keeps the joined path as its ident; resolve it here for
    // the build path exactly as the qualified bus subjects are.
    fn rewrite_binding(
        entry: &mut hale_syntax::ast::BindingEntry,
        import_renames: &[(Vec<String>, String)],
    ) {
        if !entry.topic.name.contains("::") {
            return;
        }
        let segs: Vec<&str> = entry.topic.name.split("::").collect();
        if let Some(mangled) = lookup(&segs, import_renames) {
            entry.topic.name = mangled;
        }
    }
    use hale_syntax::ast::{Block, ElseBranch, Expr, MatchArmBody, Stmt};
    fn rewrite_send_subject(
        e: &mut Expr,
        import_renames: &[(Vec<String>, String)],
    ) {
        // `source::Heartbeat <- payload;` — Expr::Path multi-segment
        // resolves to a single-segment Ident with the mangled topic
        // name so the desugar's Stmt::Send rewriter (which only
        // looks at Expr::Ident) handles it uniformly with intra-
        // seed sends.
        if let Expr::Path(qn) = e {
            if qn.segments.len() > 1 {
                let segs: Vec<&str> =
                    qn.segments.iter().map(|s| s.name.as_str()).collect();
                if let Some(mangled) = lookup(&segs, import_renames) {
                    let span = qn.span;
                    *e = Expr::Ident(Ident { name: mangled, span });
                }
            }
        }
    }
    fn walk_if(
        i: &mut hale_syntax::ast::IfStmt,
        import_renames: &[(Vec<String>, String)],
    ) {
        walk_block(&mut i.then_block, import_renames);
        if let Some(eb) = &mut i.else_block {
            match eb.as_mut() {
                ElseBranch::Else(b) => walk_block(b, import_renames),
                ElseBranch::ElseIf(nested) => walk_if(nested, import_renames),
            }
        }
    }
    fn walk_block(
        b: &mut Block,
        import_renames: &[(Vec<String>, String)],
    ) {
        for s in &mut b.stmts {
            walk_stmt(s, import_renames);
        }
        // Tail expr can't be a Send (Send is statement-only).
        let _ = &b.tail;
    }
    fn walk_stmt(
        s: &mut Stmt,
        import_renames: &[(Vec<String>, String)],
    ) {
        match s {
            Stmt::Send { subject, .. } => {
                rewrite_send_subject(subject, import_renames);
            }
            Stmt::If(i) => walk_if(i, import_renames),
            Stmt::Match(m) => {
                for arm in &mut m.arms {
                    if let MatchArmBody::Block(b) = &mut arm.body {
                        walk_block(b, import_renames);
                    }
                }
            }
            Stmt::For { body, .. } | Stmt::While { body, .. } => {
                walk_block(body, import_renames);
            }
            Stmt::Block(b) => walk_block(b, import_renames),
            _ => {}
        }
    }
    // GH #884: module nesting flattened — a qualified bus subject
    // written one brace deeper names the same topic.
    hale_syntax::ast::for_each_decl_mut(&mut program.items, &mut |item| {
        if let TopDecl::Locus(l) = item {
            for m in &mut l.members {
                match m {
                    LocusMember::Bus(b) => {
                        for bm in &mut b.members {
                            match bm {
                                BusMember::Subscribe { subject, .. } => {
                                    rewrite(subject, import_renames);
                                }
                                BusMember::Publish { subject, .. } => {
                                    rewrite(subject, import_renames);
                                }
                            }
                        }
                    }
                    LocusMember::Lifecycle(lc) => {
                        walk_block(&mut lc.body, import_renames);
                    }
                    LocusMember::Mode(md) => {
                        walk_block(&mut md.body, import_renames);
                    }
                    LocusMember::Fn(fd) => {
                        walk_block(&mut fd.body, import_renames);
                    }
                    LocusMember::Bindings(bb) => {
                        for entry in &mut bb.entries {
                            rewrite_binding(entry, import_renames);
                        }
                    }
                    _ => {}
                }
            }
        } else if let TopDecl::Fn(fd) = item {
            walk_block(&mut fd.body, import_renames);
        }
    });
}
