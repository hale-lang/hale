//! The fresh-factory shadow (F.40 phase 1.2c): two comparisons, one
//! report, keyed `escape/<fn>` and `products/<fn>`.
//!
//! **escape/** — `ownership::fresh_factories` answers which free fns
//! hand back a locus they freshly built, and which binding they hand
//! back. Its escape walk used to end in a catch-all that searched a
//! node's Debug rendering for `name: "<binding>"`, so the binding's
//! name spelled ANYWHERE in the node — a field or method name, a
//! struct-init name, a pattern, a path segment — answered "escapes".
//! 1.2c replaces the catch-all with a match over every expression
//! form that answers by identifier.
//!
//! OLD is the classification with the Debug-string catch-all, frozen
//! below as it stood before 1.2c ([`old_fresh_factories`]; the rest of
//! the walk is the same code). NEW is the producer in `src`, projected
//! onto the same value. Both are keyed by fn name, the value is
//! `(locus, returned binding)`, and both run over the program the
//! pre-pass reads: every corpus program `resolve_program` accepts
//! (merged with the stdlib, desugared). The bundled stdlib's fns are
//! the same in every merged program, so they are compared once, over
//! the stdlib merged into an empty program, and left out of the
//! per-program rows.
//!
//! **products/** — the checker's self-containment rule (GH #870) kept
//! its own mirror of the classification, `fresh_locus_factory_products`,
//! which answered with the (locus, supplied fields) products a call to
//! each factory constructs. 1.2c deletes it and the rule reads the
//! producer's rows. OLD is the mirror, frozen below as it stood
//! ([`old_mirror_products`]); NEW is what the rule now reads: the rows'
//! `products`, for the fns whose locus the bundle declares. Both run
//! in the checker's shape — each parseable corpus program parsed
//! alone, as `hale check` of a single file sees it — keyed by fn
//! name. The rule's only input from either is this map, so where the
//! two agree over a program its diagnostics are the same.
//!
//! The gate: every divergence over the corpus is classified in
//! `fixtures/shadow_fresh_factories.txt` with a note, and none is a
//! regression. Regenerate the fixture with `HALE_SHADOW_REGEN=1`,
//! then classify by hand.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use hale_graph::shadow::{gate_message, parse_fixture, program_id, Report};
use hale_syntax::ast::{
    flat_decls, Block, ElseBranch, Expr, FnDecl, IfStmt, LocusDecl, MatchArmBody, OrDisposition,
    Program, QualifiedName, Stmt, TopDecl, TypeExpr,
};
use hale_types::ownership::{fresh_factories, stdlib_mangled_for_path};
use hale_types::Bundle;

fn compute_new(program: &Program) -> Factories {
    fresh_factories(program, &[])
        .into_iter()
        .map(|(f, row)| (f, (row.locus, row.returned_binding)))
        .collect()
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/shadow_fresh_factories.txt")
}

type Factories = BTreeMap<String, (String, Option<String>)>;

fn render((locus, binding): &(String, Option<String>)) -> String {
    match binding {
        Some(b) => format!("{locus}, hands back `{b}`"),
        None => format!("{locus}, no binding"),
    }
}

/// The names the bundled stdlib declares as free fns.
fn stdlib_fn_names() -> BTreeSet<String> {
    let stdlib = hale_syntax::parse_source(hale_stdlib::AP_SOURCE).expect("stdlib parses");
    flat_decls(&stdlib.items)
        .filter_map(|d| match d {
            TopDecl::Fn(f) => Some(f.name.name.clone()),
            _ => None,
        })
        .collect()
}

/// A fn's source text, collapsed onto one line, for the fixer.
fn fn_source(program: &Program, src: &str, name: &str) -> Option<String> {
    let f: &FnDecl = flat_decls(&program.items).find_map(|d| match d {
        TopDecl::Fn(f) if f.name.name == name => Some(f),
        _ => None,
    })?;
    let (a, b) = (f.name.span.start.0 as usize, f.body.span.end.0 as usize);
    let text = src.get(a..b)?;
    let one: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(one.chars().take(400).collect())
}

fn escape_one(
    report: &mut Report,
    program_name: &str,
    merged: &Program,
    user: Option<(&Program, &str)>,
    keep: impl Fn(&str) -> bool,
) {
    let rows = |m: Factories| -> Vec<(String, String)> {
        m.iter().filter(|(k, _)| keep(k)).map(|(k, v)| (k.clone(), render(v))).collect()
    };
    let old = rows(old_fresh_factories(merged, &[]));
    let new = rows(compute_new(merged));
    report.compare_rows(
        program_name,
        &old,
        &new,
        |k| Some(format!("escape/{k}")),
        |k| Some(format!("escape/{k}")),
        |k| {
            let name = k.trim_start_matches("escape/");
            let src = user.and_then(|(p, s)| fn_source(p, s, name));
            vec![src.unwrap_or_else(|| format!("fn `{name}` (the bundled stdlib)"))]
        },
        |k| {
            vec![format!(
                "resolve_owners: a call to `{}` is owned by the binding it lands in, and the fn's returned binding is not dissolved in its frame",
                k.trim_start_matches("escape/")
            )]
        },
    );
}

type Products = BTreeMap<String, Vec<(String, Vec<String>)>>;

fn render_products(ps: &[(String, Vec<String>)]) -> String {
    ps.iter()
        .map(|(l, s)| format!("{l}{{{}}}", s.join(",")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every locus a program declares, as the rule collects them.
fn loci_of(program: &Program) -> BTreeMap<&str, &LocusDecl> {
    fn collect<'a>(items: &'a [TopDecl], out: &mut BTreeMap<&'a str, &'a LocusDecl>) {
        for item in items {
            match item {
                TopDecl::Locus(l) => {
                    out.entry(l.name.name.as_str()).or_insert(l);
                }
                TopDecl::Module(m) => collect(&m.items, out),
                _ => {}
            }
        }
    }
    let mut out = BTreeMap::new();
    collect(&program.items, &mut out);
    out
}

fn products_one(report: &mut Report, id: &str, program: &Program, src: &str) {
    let loci = loci_of(program);
    let mut programs = BTreeMap::new();
    programs.insert("app.hl".to_string(), program);
    let bundle = Bundle::new(programs);
    let old: Products = if loci.is_empty() { Products::new() } else { old_mirror_products(&bundle, &loci) };
    // What `check_self_containing_locus` now reads: the rows'
    // products, for the loci the bundle declares.
    let new: Products = if loci.is_empty() {
        Products::new()
    } else {
        fresh_factories(program, &[])
            .into_iter()
            .filter(|(_, row)| loci.contains_key(row.locus.as_str()))
            .map(|(f, row)| (f, row.products))
            .collect()
    };
    let old: Vec<(String, String)> = old.iter().map(|(k, v)| (k.clone(), render_products(v))).collect();
    let new: Vec<(String, String)> = new.iter().map(|(k, v)| (k.clone(), render_products(v))).collect();
    report.compare_rows(
        id,
        &old,
        &new,
        |k| Some(format!("products/{k}")),
        |k| Some(format!("products/{k}")),
        |k| {
            let name = k.trim_start_matches("products/");
            vec![fn_source(program, src, name).unwrap_or_else(|| format!("fn `{name}`"))]
        },
        |k| {
            // What the rule now reports on the program: every
            // self-containment diagnostic `hale check` of the file
            // gives. The old map is the new one less this row, so a
            // cycle the old rule reported is one the new rule reports.
            let reports: Vec<String> = hale_types::check_program(program)
                .into_iter()
                .filter(|d| d.message.contains("a locus cannot contain itself by value"))
                .map(|d| d.message)
                .collect();
            vec![
                format!(
                    "check_self_containing_locus: a param default that calls `{}` takes an edge to each product",
                    k.trim_start_matches("products/")
                ),
                format!("the rule reports {} self-containment diagnostic(s) on this program: {:?}", reports.len(), reports),
            ]
        },
    );
}

#[test]
fn the_fresh_factory_producers_agree_or_every_divergence_is_classified() {
    let mut report = Report::new("ownership (fresh factories)");
    let std_names = stdlib_fn_names();

    // The bundled stdlib, once.
    let empty = hale_syntax::parse_source("").expect("empty program parses");
    let resolved = hale_types::resolved::resolve_program(&empty, &[], None, None)
        .expect("the stdlib alone resolves");
    escape_one(&mut report, "<bundled stdlib>", &resolved.merged, None, |k| std_names.contains(k));

    let mut factories = 0usize;
    let mut mirror_rows = 0usize;
    for p in hale_corpus::parseable(|s| hale_syntax::parse_source(s).is_ok()) {
        let Ok(program) = hale_syntax::parse_source(&p.source) else { continue };
        let id = program_id(&p.origin, &p.source);
        // The checker's shape: the program alone, before any desugar.
        let loci = loci_of(&program);
        if !loci.is_empty() {
            let mut programs = BTreeMap::new();
            programs.insert("app.hl".to_string(), &program);
            mirror_rows += old_mirror_products(&Bundle::new(programs), &loci).len();
        }
        products_one(&mut report, &id, &program, &p.source);
        let Ok(resolved) = hale_types::resolved::resolve_program(&program, &[], None, None) else {
            continue;
        };
        factories += old_fresh_factories(&resolved.merged, &[])
            .keys()
            .filter(|k| !std_names.contains(*k))
            .count();
        escape_one(&mut report, &id, &resolved.merged, Some((&program, &p.source)), |k| {
            !std_names.contains(k)
        });
    }
    assert!(report.programs > 300, "the corpus walk is vacuous ({} programs)", report.programs);
    assert!(factories > 100, "the old side found too few user factories to compare ({factories})");
    assert!(mirror_rows > 20, "the mirror found too few factories to compare ({mirror_rows})");

    let path = fixture_path();
    let existing = std::fs::read_to_string(&path)
        .ok()
        .map(|t| parse_fixture(&t).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
        .unwrap_or_default();
    if std::env::var("HALE_SHADOW_REGEN").as_deref() == Ok("1") {
        std::fs::write(&path, report.render_fixture(&existing)).expect("write fixture");
        eprintln!("{}", report.render());
        return;
    }
    let (unexplained, stale) = report.explain(&existing);
    assert!(
        unexplained.is_empty() && stale.is_empty(),
        "{}",
        gate_message(&report, &unexplained, &stale, "crates/hale-types/tests/fixtures/shadow_fresh_factories.txt")
    );
    eprintln!(
        "shadow `{}`: {} comparisons, {} rows compared, {} divergence(s); old-side user \
         factories (escape/): {}; mirror factories (products/): {}",
        report.family,
        report.programs,
        report.rows_compared,
        report.divergences.len(),
        factories,
        mirror_rows
    );
}

/// The corpus has no fn whose answer the correction moves, so these
/// probes pin what it moves: each is `(what, body of make, old
/// accepts, new accepts)` over `locus Node { params { n: Int = 0; } }`
/// and `fn make() -> Node { … }`.
#[test]
fn the_correction_on_probes() {
    let probes: &[(&str, &str, bool, bool)] = &[
        (
            "a receiver use inside a tuple",
            "let n = Node { n: 1 }; let t = (n.n, 2); return n;",
            false,
            true,
        ),
        (
            "another value's field that spells the binding, inside an `if`",
            "let n = Node { n: 1 }; let o = Node { n: 2 }; let v = if true { o.n } else { 0 }; return n;",
            false,
            true,
        ),
        (
            "a struct-init name that spells the binding, inside a tuple",
            "let n = Node { n: 1 }; let t = (Node { n: 3 }, 1); return n;",
            false,
            true,
        ),
        (
            "a method name that spells the binding, inside a range",
            "let n = Node { n: 1 }; let o = Node { n: 2 }; for i in 0..o.n() { } return n;",
            false,
            true,
        ),
        (
            "the binding passed as an argument inside a tuple: a real escape",
            "let n = Node { n: 1 }; let t = (keep(n), 1); return n;",
            false,
            false,
        ),
        (
            "the binding returned from an `if` arm into another binding: a real escape",
            "let n = Node { n: 1 }; let m = if true { n } else { Node { n: 2 } }; return n;",
            false,
            false,
        ),
        (
            "a `match` arm that rebinds the name",
            "let n = Node { n: 1 }; let v = match 3 { n -> 1, _ -> 2 }; return n;",
            false,
            false,
        ),
        (
            "a statement form the walk does not model, inside an `if` expression",
            "let n = Node { n: 1 }; let v = if true { yield; 1 } else { 2 }; return n;",
            true,
            false,
        ),
    ];
    let mut wrong = Vec::new();
    for (what, body, old_ok, new_ok) in probes {
        let src = format!(
            "locus Node {{ params {{ n: Int = 0; }} fn n() -> Int {{ return 0; }} }}\n\
             fn keep(x: Node) -> Int {{ return 0; }}\n\
             fn make() -> Node {{ {body} }}\n"
        );
        let program = hale_syntax::parse_source(&src)
            .unwrap_or_else(|d| panic!("probe `{what}` does not parse: {d:?}"));
        let old = old_fresh_factories(&program, &[]).contains_key("make");
        let new = fresh_factories(&program, &[]).contains_key("make");
        if (old, new) != (*old_ok, *new_ok) {
            wrong.push(format!(
                "{what}: old {old} (expected {old_ok}), new {new} (expected {new_ok})"
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

// -------------------------------------------------------------------
// OLD: `compute_fresh_locus_factories` as it stood before F.40 phase
// 1.2c, frozen verbatim — the escape walk's last arm searches the
// node's Debug rendering for the binding's name.
// -------------------------------------------------------------------

fn old_fresh_factories(
    program: &Program,
    import_renames: &[(Vec<String>, String)],
) -> BTreeMap<String, (String, Option<String>)> {
    fn resolve(v: &[String], renames: &[(Vec<String>, String)]) -> Option<String> {
        if v.len() == 1 {
            return Some(v[0].clone());
        }
        let refs: Vec<&str> = v.iter().map(|s| s.as_str()).collect();
        if let Some(m) = stdlib_mangled_for_path(&refs) {
            return Some(m.to_string());
        }
        renames
            .iter()
            .find(|(p, _)| p.len() == v.len() && p.iter().zip(v).all(|(a, b)| a == b))
            .map(|(_, m)| m.clone())
    }

    fn qname(q: &QualifiedName) -> Vec<String> {
        q.segments.iter().map(|s| s.name.clone()).collect()
    }

    fn ret_locus_name(f: &FnDecl, renames: &[(Vec<String>, String)]) -> Option<String> {
        match f.ret.as_ref()? {
            TypeExpr::Named { path, .. } => resolve(&qname(path), renames),
            _ => None,
        }
    }

    #[derive(Clone, Debug)]
    enum Freshness {
        Literal,
        CallTo(String),
        Other,
    }

    fn callee_name(callee: &Expr, renames: &[(Vec<String>, String)]) -> Option<String> {
        fn segs(e: &Expr, out: &mut Vec<String>) -> bool {
            match e {
                Expr::Ident(i) => {
                    out.push(i.name.clone());
                    true
                }
                Expr::Path(q) => {
                    out.extend(q.segments.iter().map(|i| i.name.clone()));
                    true
                }
                Expr::Path2 { receiver, name, .. } | Expr::Field { receiver, name, .. } => {
                    if !segs(receiver, out) {
                        return false;
                    }
                    out.push(name.name.clone());
                    true
                }
                _ => false,
            }
        }
        let mut v = Vec::new();
        if !segs(callee, &mut v) {
            return None;
        }
        resolve(&v, renames)
    }

    fn expr_ok(e: &Expr, x: &str) -> bool {
        match e {
            Expr::Ident(i) => i.name != x,
            Expr::Literal(..) => true,
            Expr::Field { receiver, .. } => recv_ok(receiver, x),
            Expr::Call { callee, args, .. } => {
                let c = match callee.as_ref() {
                    Expr::Field { receiver, .. } => recv_ok(receiver, x),
                    Expr::Ident(i) => i.name != x,
                    other => expr_ok(other, x),
                };
                c && args.iter().all(|a| expr_ok(a, x))
            }
            Expr::Binary { left, right, .. } => expr_ok(left, x) && expr_ok(right, x),
            Expr::Unary { operand, .. } => expr_ok(operand, x),
            Expr::Index { receiver, index, .. } => recv_ok(receiver, x) && expr_ok(index, x),
            Expr::Path2 { receiver, .. } => recv_ok(receiver, x),
            Expr::Or { inner, disposition, .. } => {
                let d = match disposition {
                    OrDisposition::Substitute(e) => expr_ok(e, x),
                    OrDisposition::Fail(e, _) => expr_ok(e, x),
                    _ => true,
                };
                expr_ok(inner, x) && d
            }
            Expr::Struct { inits, .. } => inits.iter().all(|i| expr_ok(&i.value, x)),
            Expr::Array(parts, _) => parts.iter().all(|p| expr_ok(p, x)),
            Expr::Block(b) => block_ok(b, x),
            other => !format!("{:?}", other).contains(&format!("name: \"{}\"", x)),
        }
    }

    fn recv_ok(e: &Expr, x: &str) -> bool {
        match e {
            Expr::Ident(_) => true,
            Expr::Field { receiver, .. } => recv_ok(receiver, x),
            Expr::Index { receiver, index, .. } => recv_ok(receiver, x) && expr_ok(index, x),
            Expr::Call { callee, args, .. } => {
                let c = match callee.as_ref() {
                    Expr::Field { receiver, .. } => recv_ok(receiver, x),
                    other => expr_ok(other, x),
                };
                c && args.iter().all(|a| expr_ok(a, x))
            }
            other => expr_ok(other, x),
        }
    }

    fn block_ok(b: &Block, x: &str) -> bool {
        b.stmts.iter().all(|s| stmt_ok(s, x)) && b.tail.as_ref().map_or(true, |t| expr_ok(t, x))
    }

    fn stmt_ok(s: &Stmt, x: &str) -> bool {
        match s {
            Stmt::Let { name, value, .. } => name.name != x && expr_ok(value, x),
            Stmt::Assign { target, value, .. } => target.head.name != x && expr_ok(value, x),
            Stmt::Expr(e) => expr_ok(e, x),
            Stmt::While { cond, body, .. } => expr_ok(cond, x) && block_ok(body, x),
            Stmt::For { body, iter, .. } => expr_ok(iter, x) && block_ok(body, x),
            Stmt::If(i) => if_ok(i, x),
            Stmt::Return(Some(Expr::Ident(i)), _) if i.name == x => true,
            Stmt::Return(Some(e), _) => expr_ok(e, x),
            Stmt::Return(None, _) => true,
            Stmt::Break(_) | Stmt::Continue(_) => true,
            Stmt::Fail { value, .. } => expr_ok(value, x),
            _ => false,
        }
    }

    fn if_ok(i: &IfStmt, x: &str) -> bool {
        expr_ok(&i.cond, x)
            && block_ok(&i.then_block, x)
            && i.else_block.as_ref().map_or(true, |e| match e.as_ref() {
                ElseBranch::Else(b) => block_ok(b, x),
                ElseBranch::ElseIf(e) => if_ok(e, x),
            })
    }

    fn body_ok(b: &Block, x: &str) -> bool {
        for s in &b.stmts {
            match s {
                Stmt::Let { name, value, .. } if name.name == x => {
                    if !expr_ok(value, x) {
                        return false;
                    }
                }
                other => {
                    if !stmt_ok(other, x) {
                        return false;
                    }
                }
            }
        }
        b.tail.as_ref().map_or(true, |t| match t.as_ref() {
            Expr::Ident(i) if i.name == x => true,
            e => expr_ok(e, x),
        })
    }

    fn collect(
        b: &Block,
        rets: &mut Vec<Expr>,
        lets: &mut Vec<(String, Freshness)>,
        l: &str,
        renames: &[(Vec<String>, String)],
    ) {
        for s in &b.stmts {
            match s {
                Stmt::Return(Some(e), _) => rets.push(e.clone()),
                Stmt::Let { name, value, .. } => {
                    let fr = match value {
                        Expr::Struct { path, .. }
                            if resolve(&qname(path), renames).as_deref() == Some(l) =>
                        {
                            Freshness::Literal
                        }
                        Expr::Call { callee, .. } => match callee_name(callee, renames) {
                            Some(n) => Freshness::CallTo(n),
                            None => Freshness::Other,
                        },
                        _ => Freshness::Other,
                    };
                    lets.push((name.name.clone(), fr));
                }
                Stmt::While { body, .. } | Stmt::For { body, .. } => {
                    collect(body, rets, lets, l, renames)
                }
                Stmt::If(i) => {
                    collect(&i.then_block, rets, lets, l, renames);
                    let mut cur = i.else_block.as_deref();
                    while let Some(eb) = cur {
                        match eb {
                            ElseBranch::Else(bb) => {
                                collect(bb, rets, lets, l, renames);
                                cur = None;
                            }
                            ElseBranch::ElseIf(ei) => {
                                collect(&ei.then_block, rets, lets, l, renames);
                                cur = ei.else_block.as_deref();
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(t) = &b.tail {
            rets.push((**t).clone());
        }
    }

    let mut out: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();
    loop {
        let mut added = false;
        for item in flat_decls(&program.items) {
            let TopDecl::Fn(f) = item else { continue };
            if out.contains_key(&f.name.name) {
                continue;
            }
            let Some(l) = ret_locus_name(f, import_renames) else {
                continue;
            };
            let mut rets = Vec::new();
            let mut lets = Vec::new();
            collect(&f.body, &mut rets, &mut lets, &l, import_renames);
            if rets.is_empty() {
                continue;
            }
            let mut fresh_name: Option<String> = None;
            let mut ok = true;
            for r in &rets {
                match r {
                    Expr::Struct { path, .. }
                        if resolve(&qname(path), import_renames).as_deref() == Some(l.as_str()) => {}
                    Expr::Call { callee, .. }
                        if callee_name(callee, import_renames)
                            .and_then(|c| out.get(&c).cloned())
                            .map(|(cl, _)| cl == l)
                            .unwrap_or(false) => {}
                    Expr::Ident(i) => match &fresh_name {
                        None => fresh_name = Some(i.name.clone()),
                        Some(n) if *n == i.name => {}
                        Some(_) => {
                            ok = false;
                            break;
                        }
                    },
                    _ => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                continue;
            }
            if let Some(x) = &fresh_name {
                let bindings: Vec<&(String, Freshness)> =
                    lets.iter().filter(|(n, _)| n == x).collect();
                if bindings.len() != 1 {
                    continue;
                }
                let fresh_binding = match &bindings[0].1 {
                    Freshness::Literal => true,
                    Freshness::CallTo(c) => out.get(c).map(|(cl, _)| *cl == l).unwrap_or(false),
                    Freshness::Other => false,
                };
                if !fresh_binding || !body_ok(&f.body, x) {
                    continue;
                }
            }
            out.insert(f.name.name.clone(), (l, fresh_name));
            added = true;
        }
        if !added {
            break;
        }
    }
    out
}

// -------------------------------------------------------------------
// OLD (products/): the checker's `fresh_locus_factory_products` as it
// stood before F.40 phase 1.2c, frozen verbatim.
// -------------------------------------------------------------------

type ContainmentState = (String, Vec<String>);

fn plain_callee_name(callee: &Expr) -> Option<&str> {
    match callee {
        Expr::Ident(i) => Some(i.name.as_str()),
        Expr::Path(q) if q.segments.len() == 1 => Some(q.segments[0].name.as_str()),
        _ => None,
    }
}

fn old_mirror_products(
    bundle: &Bundle<'_>,
    loci: &BTreeMap<&str, &LocusDecl>,
) -> BTreeMap<String, Vec<ContainmentState>> {
    fn literal_state(e: &Expr, locus: &str) -> Option<ContainmentState> {
        let Expr::Struct { path, inits, .. } = e else { return None };
        if path.segments.len() != 1 || path.segments[0].name != locus {
            return None;
        }
        let mut supplied: Vec<String> = inits.iter().map(|i| i.name.name.clone()).collect();
        supplied.sort();
        supplied.dedup();
        Some((locus.to_string(), supplied))
    }

    fn arm_states(
        e: &Expr,
        locus: &str,
        lets: &[(&str, &Expr)],
        known: &BTreeMap<String, Vec<ContainmentState>>,
    ) -> Option<Vec<ContainmentState>> {
        if let Some(s) = literal_state(e, locus) {
            return Some(vec![s]);
        }
        if let Expr::Call { callee, .. } = e {
            let states = known.get(plain_callee_name(callee)?)?;
            if states.iter().all(|(l, _)| l == locus) {
                return Some(states.clone());
            }
            return None;
        }
        if let Expr::Ident(i) = e {
            let bound: Vec<&Expr> =
                lets.iter().filter(|(n, _)| *n == i.name).map(|(_, v)| *v).collect();
            if bound.len() != 1 {
                return None;
            }
            return arm_states(bound[0], locus, &[], known);
        }
        None
    }

    fn collect_returns<'a>(
        b: &'a Block,
        rets: &mut Vec<&'a Expr>,
        lets: &mut Vec<(&'a str, &'a Expr)>,
    ) -> bool {
        for s in &b.stmts {
            match s {
                Stmt::Return(Some(e), _) => rets.push(e),
                Stmt::Let { name, value, .. } => lets.push((name.name.as_str(), value)),
                Stmt::If(i) => {
                    if !if_returns(i, rets, lets) {
                        return false;
                    }
                }
                Stmt::Match(m) => {
                    for arm in &m.arms {
                        match &arm.body {
                            MatchArmBody::Block(bb) => {
                                if !collect_returns(bb, rets, lets) {
                                    return false;
                                }
                            }
                            MatchArmBody::Expr(_) => {}
                        }
                    }
                }
                Stmt::For { body, .. } | Stmt::While { body, .. } | Stmt::Block(body) => {
                    if !collect_returns(body, rets, lets) {
                        return false;
                    }
                }
                Stmt::Return(None, _)
                | Stmt::LetTuple { .. }
                | Stmt::Assign { .. }
                | Stmt::Expr(_)
                | Stmt::Break(_)
                | Stmt::Continue(_)
                | Stmt::Fail { .. }
                | Stmt::Yield(_)
                | Stmt::Terminate(_)
                | Stmt::Reperspective { .. }
                | Stmt::Recovery { .. }
                | Stmt::Violate { .. }
                | Stmt::Send { .. } => {}
                _ => return false,
            }
        }
        if let Some(t) = &b.tail {
            rets.push(t);
        }
        true
    }

    fn if_returns<'a>(
        i: &'a IfStmt,
        rets: &mut Vec<&'a Expr>,
        lets: &mut Vec<(&'a str, &'a Expr)>,
    ) -> bool {
        if !collect_returns(&i.then_block, rets, lets) {
            return false;
        }
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => collect_returns(b, rets, lets),
            Some(ElseBranch::ElseIf(nested)) => if_returns(nested, rets, lets),
            None => true,
        }
    }

    let mut fns: BTreeMap<&str, &FnDecl> = BTreeMap::new();
    for program in bundle.programs.values() {
        for item in flat_decls(&program.items) {
            if let TopDecl::Fn(f) = item {
                fns.entry(f.name.name.as_str()).or_insert(f);
            }
        }
    }
    let mut out: BTreeMap<String, Vec<ContainmentState>> = BTreeMap::new();
    loop {
        let mut added = false;
        for (name, f) in &fns {
            if out.contains_key(*name) {
                continue;
            }
            let locus = match f.ret.as_ref() {
                Some(TypeExpr::Named { path, .. }) if path.segments.len() == 1 => {
                    path.segments[0].name.as_str()
                }
                _ => continue,
            };
            if !loci.contains_key(locus) {
                continue;
            }
            let mut rets: Vec<&Expr> = Vec::new();
            let mut lets: Vec<(&str, &Expr)> = Vec::new();
            if !collect_returns(&f.body, &mut rets, &mut lets) {
                continue;
            }
            let mut states: Vec<ContainmentState> = Vec::new();
            let mut fresh = !rets.is_empty();
            for r in &rets {
                match arm_states(r, locus, &lets, &out) {
                    Some(s) => states.extend(s),
                    None => {
                        fresh = false;
                        break;
                    }
                }
            }
            if !fresh || states.is_empty() {
                continue;
            }
            states.sort();
            states.dedup();
            out.insert((*name).to_string(), states);
            added = true;
        }
        if !added {
            break;
        }
    }
    out
}
