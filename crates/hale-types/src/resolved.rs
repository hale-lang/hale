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
//! The sequence is codegen's former one, moved here unchanged: the
//! same passes, in the same order, over the same inputs. The one
//! addition is the mint over the merged program before the pre-pass,
//! so every stdlib and desugar-generated node has its identity before
//! the pre-pass numbers anything (the mint keeps the ids the bundle
//! already carries and continues the counter, so the pre-pass then
//! finds every `Struct` and `Call` numbered).
//!
//! Today the verbs still run `json_gen`, api injection and sync
//! inference before the check, and this step re-runs the idempotent
//! ones on its own clone, exactly as codegen did. Phase 2 makes this
//! the only place the sequence runs, before the check.

use std::collections::BTreeMap;

use hale_syntax::ast::{LocusMember, Program, TopDecl, TypeExpr};

use crate::ownership::OwnerTable;
use crate::snapshot::Snapshot;

/// The program codegen lowers, and the tables the frontend derives over it.
pub struct ResolvedProgram {
    /// The user's program after the codegen-shape desugars, before the
    /// stdlib merge. Codegen's tier-1 bus-inert scan reads it.
    pub user: Program,
    /// `user` with the bundled stdlib's declarations appended, unit
    /// returns normalized, construction aliases resolved and the
    /// omitted `run` synthesized: what lowering walks.
    pub merged: Program,
    /// Every site's identity, minted over `merged` after the desugars
    /// (ids the bundle already minted are kept; the counter continues).
    pub snapshot: Snapshot,
    pub owner_table: OwnerTable,
    /// Fresh factories, extended by the carrier-return fold.
    pub fresh_locus_factories: BTreeMap<String, (String, Option<String>)>,
    /// What producing the envelope cost, so a build's phase timing
    /// (`HALE_TIME`, `BuildOptions::time_phases`) can report the
    /// resolve step beside the phases codegen times itself.
    pub resolved_in: std::time::Duration,
}

/// Resolve `program` into the envelope codegen lowers.
///
/// `import_renames` is the per-build path-rename table for cross-seed
/// imports (see `hale_codegen::build_executable_with_options`); `api`
/// and `api_roles` are the build's `--api` path and the roles its
/// environment binds. The error is the message codegen reports as
/// `CodegenError::Unsupported`: a refused `--api` injection, or a
/// bundled stdlib that does not parse.
pub fn resolve_program(
    program: &Program,
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
    let snapshot = crate::snapshot::mint([("program", &mut merged)], &[]);

    // GH #921 A2: the ownership pre-pass, over the merged and
    // desugared program and before anything borrows it. It numbers
    // every locus-producing expression node (the only mutation it
    // makes) and derives an owner for each from syntactic position,
    // using the same fresh-factory fixpoint lowering uses — extended
    // to the carrier returns that fixpoint misses.
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
        crate::ownership::fresh_factories(&merged, import_renames)
            .into_iter()
            .map(|(f, row)| (f, (row.locus, row.returned_binding)))
            .collect();
    let mut owner_table = crate::ownership::resolve_owners(
        &mut merged,
        &fresh_locus_factories,
        import_renames,
    );
    // F.40 phase 1.2b: and what each `let` needs to know about its
    // own binding, one row per binding site, keyed by the identity
    // minted above.
    crate::ownership::resolve_binding_facts(&merged, &mut owner_table);
    for (fname, locus) in owner_table.extended_fresh_factories() {
        // A carrier return hands back an ARM's value, so there is no
        // single returned binding to name: `None`, the same as a fn
        // whose every `return` is a literal.
        fresh_locus_factories
            .entry(fname.clone())
            .or_insert_with(|| (locus.clone(), None));
    }

    Ok(ResolvedProgram {
        user,
        merged,
        snapshot,
        owner_table,
        fresh_locus_factories,
        resolved_in: t_start.elapsed(),
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
    fn lookup<'a>(
        segs: &[&str],
        import_renames: &'a [(Vec<String>, String)],
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
