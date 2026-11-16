//! The mutators: each takes a candidate's files and a seeded RNG and
//! returns a new candidate, or `None` when the program has no site the
//! mutator can act on.
//!
//! A mutator edits SOURCE TEXT at the spans the parser recorded (there is
//! no AST printer; `hale fmt` is a token-stream formatter), so a mutation
//! keeps every line it does not touch byte for byte. A mutation need not
//! be correct: the gate checks every candidate first and discards the
//! ones the check refuses, so a mutator is free to guess — what it must
//! never do is panic. Each one says in its description what it did and
//! where, which is the mutation chain a divergence report carries.
//!
//! ## Adding a mutator
//!
//! Write `fn my_mutator(files: &Files, rng: &mut Rng) -> Option<(Files,
//! String)>`: parse `main/main.hl` with [`parsed`], collect the sites you
//! can act on, [`Rng::pick`] one, splice the text with [`splice`] (and
//! [`append`] for a declaration you add), and return the new files with a
//! one-line description naming the site. Then add it to [`MUTATORS`]. Name
//! anything you introduce with a `__pg`/`__Pg` prefix so it cannot collide
//! with the program's own names. `mutators_reach_check_clean_programs`
//! (in `parity_gate.rs`) fails until the new mutator produces at least one
//! check-clean candidate over the sample, so a mutator that only ever
//! writes broken programs is caught at once.


use hale_syntax::ast::*;
use hale_syntax::lexer::{lex, TokenKind};
use hale_syntax::Span;

use crate::gate::Files;

pub const MAIN: &str = "main/main.hl";
pub const SPLIT: &str = "main/split.hl";
pub const LIB: &str = "main/pglib/lib.hl";
pub const ALIAS: &str = "pglib";

// --- determinism -----------------------------------------------------

/// SplitMix64: small, seedable, and the same on every platform.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next_u64() % n as u64) as usize
        }
    }
    pub fn pick<'a, T>(&mut self, v: &'a [T]) -> Option<&'a T> {
        if v.is_empty() {
            None
        } else {
            Some(&v[self.below(v.len())])
        }
    }
}

/// FNV-1a: a stable hash of a program's text, so a candidate's identity
/// does not depend on where its seed sits in the corpus.
pub fn fnv(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

// --- text editing ----------------------------------------------------

/// Replace each `(start, end, text)` range of `src`. The ranges must not
/// overlap; they are applied back to front so earlier offsets hold.
pub fn splice(src: &str, mut edits: Vec<(usize, usize, String)>) -> Option<String> {
    edits.sort_by_key(|e| std::cmp::Reverse(e.0));
    let mut out = src.to_string();
    let mut floor = usize::MAX;
    for (s, e, t) in edits {
        if s > e || e > out.len() || e > floor || !src.is_char_boundary(s) || !src.is_char_boundary(e) {
            return None;
        }
        out.replace_range(s..e, &t);
        floor = s;
    }
    Some(out)
}

/// `src` with `decl` appended as a top-level declaration.
pub fn append(src: &str, decl: &str) -> String {
    let mut out = src.trim_end().to_string();
    out.push_str("\n\n");
    out.push_str(decl.trim());
    out.push('\n');
    out
}

fn text(src: &str, span: Span) -> Option<&str> {
    src.get(span.start.as_usize()..span.end.as_usize())
}

/// The end of a statement whose span may stop before its `;`.
fn past_semicolon(src: &str, end: usize) -> usize {
    let rest = &src[end..];
    let trimmed = rest.trim_start();
    if trimmed.starts_with(';') {
        end + (rest.len() - trimmed.len()) + 1
    } else {
        end
    }
}

pub fn parsed(files: &Files) -> Option<(String, Program)> {
    let src = files.get(MAIN)?.clone();
    let program = hale_syntax::parse_source(&src).ok()?;
    Some((src, program))
}

fn with_main(files: &Files, src: String) -> Files {
    let mut out = files.clone();
    out.insert(MAIN.to_string(), src);
    out
}

// --- walking ---------------------------------------------------------

/// A fn the program declares, with where it lives.
pub struct FnSite<'a> {
    pub f: &'a FnDecl,
    /// The locus it is a member of, when it is one.
    pub locus: Option<&'a LocusDecl>,
}

pub fn fns(program: &Program) -> Vec<FnSite<'_>> {
    let mut out = Vec::new();
    fn walk<'a>(items: &'a [TopDecl], out: &mut Vec<FnSite<'a>>) {
        for item in items {
            match item {
                TopDecl::Fn(f) => out.push(FnSite { f, locus: None }),
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        if let LocusMember::Fn(f) = m {
                            out.push(FnSite { f, locus: Some(l) });
                        }
                    }
                }
                TopDecl::Module(m) => walk(&m.items, out),
                _ => {}
            }
        }
    }
    walk(&program.items, &mut out);
    out
}

/// Every body a program runs: fns, and a locus's lifecycle, mode and
/// failure bodies; with whether it is a `fallible(E)` fn's.
fn bodies(program: &Program) -> Vec<(&Block, bool)> {
    let mut out = Vec::new();
    for item in &program.items {
        match item {
            TopDecl::Fn(f) => out.push((&f.body, f.fallible.is_some())),
            TopDecl::Locus(l) => {
                for m in &l.members {
                    match m {
                        LocusMember::Fn(f) => out.push((&f.body, f.fallible.is_some())),
                        LocusMember::Lifecycle(d) => out.push((&d.body, false)),
                        LocusMember::Mode(d) => out.push((&d.body, false)),
                        LocusMember::Failure(d) => out.push((&d.body, false)),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// An expression with what a mutator needs around it.
#[derive(Clone, Copy)]
pub struct ExprSite<'a> {
    pub e: &'a Expr,
    /// Where a statement can be inserted to run just before the one
    /// holding this expression (the start of that statement, or of the
    /// block tail it is in).
    pub before: usize,
    /// Whether it sits where a value is read (not a callee, not the
    /// left of a `::`).
    pub value: bool,
    /// Whether the fn around it is `fallible(E)` (where `or fail` is
    /// legal).
    pub fallible: bool,
}

fn walk_block<'a>(b: &'a Block, out: &mut Vec<ExprSite<'a>>) {
    for s in &b.stmts {
        let before = s.span().start.as_usize();
        walk_stmt(s, before, out);
    }
    if let Some(t) = &b.tail {
        walk_expr(t, t.span().start.as_usize(), true, out);
    }
}

fn walk_stmt<'a>(s: &'a Stmt, before: usize, out: &mut Vec<ExprSite<'a>>) {
    match s {
        Stmt::Let { value, .. } | Stmt::LetTuple { value, .. } => walk_expr(value, before, true, out),
        Stmt::Assign { value, .. } => walk_expr(value, before, true, out),
        Stmt::If(i) => walk_if(i, before, out),
        Stmt::Match(m) => walk_match(m, before, out),
        Stmt::For { iter, body, .. } => {
            walk_expr(iter, before, true, out);
            walk_block(body, out);
        }
        Stmt::While { cond, body, .. } => {
            walk_expr(cond, before, true, out);
            walk_block(body, out);
        }
        Stmt::Return(Some(e), _) => walk_expr(e, before, true, out),
        Stmt::Fail { value, .. } => walk_expr(value, before, true, out),
        Stmt::Block(b) => walk_block(b, out),
        Stmt::Send { value, .. } => walk_expr(value, before, true, out),
        Stmt::Expr(e) => walk_expr(e, before, true, out),
        _ => {}
    }
}

fn walk_if<'a>(i: &'a IfStmt, before: usize, out: &mut Vec<ExprSite<'a>>) {
    walk_expr(&i.cond, before, true, out);
    walk_block(&i.then_block, out);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => walk_block(b, out),
        Some(ElseBranch::ElseIf(e)) => walk_if(e, before, out),
        None => {}
    }
}

fn walk_match<'a>(m: &'a MatchStmt, before: usize, out: &mut Vec<ExprSite<'a>>) {
    walk_expr(&m.scrutinee, before, true, out);
    for arm in &m.arms {
        match &arm.body {
            MatchArmBody::Expr(e) => walk_expr(e, before, true, out),
            MatchArmBody::Block(b) => walk_block(b, out),
        }
    }
}

fn walk_expr<'a>(e: &'a Expr, before: usize, value: bool, out: &mut Vec<ExprSite<'a>>) {
    out.push(ExprSite { e, before, value, fallible: false });
    match e {
        Expr::Binary { left, right, .. } => {
            walk_expr(left, before, true, out);
            walk_expr(right, before, true, out);
        }
        Expr::Unary { operand, .. } => walk_expr(operand, before, true, out),
        Expr::Call { callee, args, .. } => {
            walk_expr(callee, before, false, out);
            for a in args {
                walk_expr(a, before, true, out);
            }
        }
        Expr::Field { receiver, .. } => walk_expr(receiver, before, true, out),
        Expr::Index { receiver, index, .. } => {
            walk_expr(receiver, before, true, out);
            walk_expr(index, before, true, out);
        }
        Expr::Path2 { receiver, .. } => walk_expr(receiver, before, false, out),
        Expr::Tuple(xs, _) | Expr::Array(xs, _) => {
            for x in xs {
                walk_expr(x, before, true, out);
            }
        }
        Expr::Struct { inits, .. } => {
            for i in inits {
                walk_expr(&i.value, before, true, out);
            }
        }
        Expr::Block(b) => walk_block(b, out),
        Expr::If(i) => walk_if(i, before, out),
        Expr::Match(m) => walk_match(m, before, out),
        Expr::Or { inner, disposition, .. } => {
            walk_expr(inner, before, true, out);
            if let OrDisposition::Substitute(d) | OrDisposition::Fail(d, _) = disposition {
                walk_expr(d, before, true, out);
            }
        }
        Expr::Range { lo, hi, .. } => {
            walk_expr(lo, before, true, out);
            walk_expr(hi, before, true, out);
        }
        Expr::ArrayRepeat { val, .. } => walk_expr(val, before, true, out),
        _ => {}
    }
}

pub fn exprs(program: &Program) -> Vec<ExprSite<'_>> {
    let mut out = Vec::new();
    for (b, fallible) in bodies(program) {
        let from = out.len();
        walk_block(b, &mut out);
        for s in &mut out[from..] {
            s.fallible = fallible;
        }
    }
    out
}

/// The returns in a block, nested blocks included (not nested fns:
/// there are none).
fn returns<'a>(b: &'a Block, out: &mut Vec<(&'a Expr, Span)>) {
    let mut stmts: Vec<&Stmt> = b.stmts.iter().collect();
    while let Some(s) = stmts.pop() {
        match s {
            Stmt::Return(Some(e), span) => out.push((e, *span)),
            Stmt::If(i) => if_returns(i, out),
            Stmt::Match(m) => {
                for arm in &m.arms {
                    if let MatchArmBody::Block(b) = &arm.body {
                        returns(b, out);
                    }
                }
            }
            Stmt::For { body, .. } | Stmt::While { body, .. } => returns(body, out),
            Stmt::Block(b) => returns(b, out),
            _ => {}
        }
    }
}

fn if_returns<'a>(i: &'a IfStmt, out: &mut Vec<(&'a Expr, Span)>) {
    returns(&i.then_block, out);
    match i.else_block.as_deref() {
        Some(ElseBranch::Else(b)) => returns(b, out),
        Some(ElseBranch::ElseIf(e)) => if_returns(e, out),
        None => {}
    }
}

/// A fn's final value: the `return e;` that ends its body, or its tail.
/// The range is what a mutation rewriting the final value replaces.
struct Final<'a> {
    value: &'a Expr,
    start: usize,
    end: usize,
    was_return: bool,
}

fn final_value<'a>(src: &str, f: &'a FnDecl) -> Option<Final<'a>> {
    f.ret.as_ref()?;
    if let Some(t) = &f.body.tail {
        let sp = t.span();
        return Some(Final { value: t, start: sp.start.as_usize(), end: sp.end.as_usize(), was_return: false });
    }
    match f.body.stmts.last()? {
        Stmt::Return(Some(e), sp) => Some(Final {
            value: e,
            start: sp.start.as_usize(),
            end: past_semicolon(src, sp.end.as_usize()),
            was_return: true,
        }),
        _ => None,
    }
}

fn fn_desc(site: &FnSite) -> String {
    match site.locus {
        Some(l) => format!("{}.{}", l.name.name, site.f.name.name),
        None => site.f.name.name.clone(),
    }
}

/// A fn the mutators may rewrite: written in the program (not an
/// `@ffi` stub, which has no body).
fn rewritable(f: &FnDecl) -> bool {
    f.ffi.is_none()
}

// --- the mutators ----------------------------------------------------

pub type Apply = fn(&Files, &mut Rng) -> Option<(Files, String)>;

pub struct Mutator {
    pub name: &'static str,
    pub apply: Apply,
}

pub const MUTATORS: &[Mutator] = &[
    Mutator { name: "return_to_tail", apply: return_to_tail },
    Mutator { name: "tail_to_return", apply: tail_to_return },
    Mutator { name: "tail_shape", apply: tail_shape },
    Mutator { name: "return_in_loop", apply: return_in_loop },
    Mutator { name: "return_via_let", apply: return_via_let },
    Mutator { name: "generic_identity", apply: generic_identity },
    Mutator { name: "generic_box", apply: generic_box },
    Mutator { name: "generic_param", apply: generic_param },
    Mutator { name: "seed_split", apply: seed_split },
    Mutator { name: "import_alias", apply: import_alias },
    Mutator { name: "element_probe", apply: element_probe },
    Mutator { name: "call_via_fn_value", apply: call_via_fn_value },
    Mutator { name: "call_via_wrapper", apply: call_via_wrapper },
    Mutator { name: "method_to_free", apply: method_to_free },
    Mutator { name: "interface_dispatch", apply: interface_dispatch },
    Mutator { name: "or_vary", apply: or_vary },
    Mutator { name: "fallible_wrap", apply: fallible_wrap },
];

/// `return e;` ending a fn's body becomes the tail `e`.
fn return_to_tail(files: &Files, rng: &mut Rng) -> Option<(Files, String)> {
    let (src, program) = parsed(files)?;
    let sites: Vec<_> = fns(&program)
        .into_iter()
        .filter(|s| rewritable(s.f))
        .filter_map(|s| final_value(&src, s.f).filter(|f| f.was_return).map(|fv| (s, fv)))
        .collect();
    let (site, fv) = rng.pick(&sites)?;
    let e = text(&src, fv.value.span())?;
    let out = splice(&src, vec![(fv.start, fv.end, e.to_string())])?;
    Some((with_main(files, out), format!("return_to_tail({})", fn_desc(site))))
}

/// A fn's tail `e` becomes `return e;`.
fn tail_to_return(files: &Files, rng: &mut Rng) -> Option<(Files, String)> {
    let (src, program) = parsed(files)?;
    let sites: Vec<_> = fns(&program)
        .into_iter()
        .filter(|s| rewritable(s.f))
        .filter_map(|s| final_value(&src, s.f).filter(|f| !f.was_return).map(|fv| (s, fv)))
        .collect();
    let (site, fv) = rng.pick(&sites)?;
    let e = text(&src, fv.value.span())?;
    let out = splice(&src, vec![(fv.start, fv.end, format!("return {e};"))])?;
    Some((with_main(files, out), format!("tail_to_return({})", fn_desc(site))))
}

/// A fn's final value `e` becomes a tail of another shape: an `if`, a
/// `match`, a block, or a `return` behind an `if`.
fn tail_shape(files: &Files, rng: &mut Rng) -> Option<(Files, String)> {
    let (src, program) = parsed(files)?;
    let sites: Vec<_> = fns(&program)
        .into_iter()
        .filter(|s| rewritable(s.f))
        .filter_map(|s| final_value(&src, s.f).map(|fv| (s, fv)))
        .collect();
    let (site, fv) = rng.pick(&sites)?;
    let e = text(&src, fv.value.span())?;
    let shapes = ["if", "match", "block", "if_return", "match_return"];
    let shape = *rng.pick(&shapes)?;
    let new = match shape {
        "if" => format!("if true {{ {e} }} else {{ {e} }}"),
        "match" => format!("match 0 {{\n        0 -> {e},\n        _ -> {e},\n    }}"),
        "block" => format!("{{ {e} }}"),
        "if_return" => format!("if true {{\n        return {e};\n    }}\n    return {e};"),
        _ => format!("match 1 {{\n        1 -> {{ return {e}; }}\n        _ -> {{ return {e}; }}\n    }}"),
    };
    let out = splice(&src, vec![(fv.start, fv.end, new)])?;
    Some((with_main(files, out), format!("tail_shape({}, {shape})", fn_desc(site))))
}

/// A fn's final value is returned from inside a loop (an early return
/// in a loop), with the plain return after it for the checker's
/// definite-return rule.
fn return_in_loop(files: &Files, rng: &mut Rng) -> Option<(Files, String)> {
    let (src, program) = parsed(files)?;
    let sites: Vec<_> = fns(&program)
        .into_iter()
        .filter(|s| rewritable(s.f))
        .filter_map(|s| final_value(&src, s.f).map(|fv| (s, fv)))
        .collect();
    let (site, fv) = rng.pick(&sites)?;
    let e = text(&src, fv.value.span())?;
    let loops = ["for", "while", "for_if"];
    let shape = *rng.pick(&loops)?;
    let new = match shape {
        "for" => format!("for __pg_i in 0..1 {{\n        return {e};\n    }}\n    return {e};"),
        "while" => format!(
            "let mut __pg_n = 0;\n    while __pg_n < 3 {{\n        __pg_n = __pg_n + 1;\n        if __pg_n == 2 {{\n            return {e};\n        }}\n    }}\n    return {e};"
        ),
        _ => format!(
            "for __pg_i in 0..4 {{\n        if __pg_i == 1 {{\n            return {e};\n        }}\n    }}\n    return {e};"
        ),
    };
    let out = splice(&src, vec![(fv.start, fv.end, new)])?;
    Some((with_main(files, out), format!("return_in_loop({}, {shape})", fn_desc(site))))
}

/// `return e;` anywhere in a fn becomes `let __pg_r = e; return __pg_r;`.
fn return_via_let(files: &Files, rng: &mut Rng) -> Option<(Files, String)> {
    let (src, program) = parsed(files)?;
    let mut sites: Vec<(String, &Expr, Span)> = Vec::new();
    for s in fns(&program).into_iter().filter(|s| rewritable(s.f)) {
        let mut rs = Vec::new();
        returns(&s.f.body, &mut rs);
        for (e, sp) in rs {
            sites.push((fn_desc(&s), e, sp));
        }
    }
    let (name, e, sp) = rng.pick(&sites)?;
    let et = text(&src, e.span())?;
    let mutable = if rng.below(2) == 0 { "" } else { "mut " };
    let new = format!("let {mutable}__pg_r = {et};\n    return __pg_r");
    let out = splice(&src, vec![(sp.start.as_usize(), e.span().end.as_usize(), new)])?;
    Some((with_main(files, out), format!("return_via_let({name}, {mutable}binding)")))
}

/// Value sites worth wrapping: read as a value, and not a bare path (a
/// path in value position is usually an enum variant or a constant,
/// which the wrap would only rename).
fn value_sites<'a>(program: &'a Program) -> Vec<ExprSite<'a>> {
    exprs(program)
        .into_iter()
        .filter(|s| s.value)
        .filter(|s| !matches!(s.e, Expr::KwSelf(_) | Expr::Range { .. } | Expr::Block(_)))
        .collect()
}

fn describe(src: &str, e: &Expr) -> String {
    let t = text(src, e.span()).unwrap_or("?");
    let one: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.len() > 40 {
        format!("`{}…`", &one[..one.char_indices().nth(40).map(|(i, _)| i).unwrap_or(one.len())])
    } else {
        format!("`{one}`")
    }
}

/// A value `e` becomes `__pg_id(e)`, through a generic identity fn.
fn generic_identity(files: &Files, rng: &mut Rng) -> Option<(Files, String)> {
    let (src, program) = parsed(files)?;
    let sites = value_sites(&program);
    let site = rng.pick(&sites)?;
    let sp = site.e.span();
    let e = text(&src, sp)?;
    let out = splice(&src, vec![(sp.start.as_usize(), sp.end.as_usize(), format!("__pg_id({e})"))])?;
    let out = append(&out, "fn __pg_id<T>(x: T) -> T {\n    return x;\n}");
    Some((with_main(files, out), format!("generic_identity({})", describe(&src, site.e))))
}

/// A value `e` travels through a generic record: `__pg_box(e).v`, or a
/// binding of one. The record is built where its type arguments are
/// declared, a generic fn's return slot (`-> __PgBox<T>`): a literal
/// spelled with the template name anywhere else is refused by the check
/// (GH #1965), so `__PgBox { v: e }.v` would test nothing.
fn generic_box(files: &Files, rng: &mut Rng) -> Option<(Files, String)> {
    let (src, program) = parsed(files)?;
    let sites = value_sites(&program);
    let site = rng.pick(&sites)?;
    let sp = site.e.span();
    let e = text(&src, sp)?;
    let bound = rng.below(2) == 0;
    let edits = if bound {
        vec![
            (site.before, site.before, format!("let __pg_b = __pg_box({e});\n    ")),
            (sp.start.as_usize(), sp.end.as_usize(), "__pg_b.v".to_string()),
        ]
    } else {
        vec![(sp.start.as_usize(), sp.end.as_usize(), format!("__pg_box({e}).v"))]
    };
    let out = splice(&src, edits)?;
    let out = append(
        &out,
        "type __PgBox<T> {\n    v: T;\n}\n\nfn __pg_box<T>(x: T) -> __PgBox<T> {\n    return __PgBox { v: x };\n}",
    );
    let how = if bound { "bound" } else { "inline" };
    Some((with_main(files, out), format!("generic_box({}, {how})", describe(&src, site.e))))
}

/// A free fn's parameter type becomes a type parameter: `fn f(x: P)` is
/// `fn f<__PgT>(x: __PgT)` (and its return type too when it is `P`).
fn generic_param(files: &Files, rng: &mut Rng) -> Option<(Files, String)> {
    let (src, program) = parsed(files)?;
    let sites: Vec<_> = fns(&program)
        .into_iter()
        .filter(|s| s.locus.is_none() && rewritable(s.f) && s.f.generics.is_empty() && !s.f.params.is_empty())
        .filter(|s| s.f.name.name != "main")
        .collect();
    let site = rng.pick(&sites)?;
    let f = site.f;
    let i = rng.below(f.params.len());
    let p = &f.params[i];
    let pty = text(&src, p.ty.span())?.to_string();
    let mut edits = vec![
        (f.name.span.end.as_usize(), f.name.span.end.as_usize(), "<__PgT>".to_string()),
        (p.ty.span().start.as_usize(), p.ty.span().end.as_usize(), "__PgT".to_string()),
    ];
    if let Some(r) = &f.ret {
        if text(&src, r.span()) == Some(pty.as_str()) {
            edits.push((r.span().start.as_usize(), r.span().end.as_usize(), "__PgT".to_string()));
        }
    }
    let out = splice(&src, edits)?;
    Some((with_main(files, out), format!("generic_param({}, {}: {pty})", f.name.name, p.name.name)))
}

/// A top-level declaration's span, with the annotations written before
/// it (`@form(vec)`, `@hot`), which its own span may not cover.
fn decl_span(item: &TopDecl) -> Option<Span> {
    match item {
        TopDecl::Fn(f) => Some(f.span),
        TopDecl::Locus(l) => Some(l.span),
        TopDecl::Type(t) => Some(t.span),
        TopDecl::Const(c) => Some(c.span),
        TopDecl::Interface(i) => Some(i.span),
        _ => None,
    }
}

fn decl_name(item: &TopDecl) -> Option<&str> {
    match item {
        TopDecl::Fn(f) => Some(&f.name.name),
        TopDecl::Locus(l) => Some(&l.name.name),
        TopDecl::Type(t) => Some(&t.name.name),
        TopDecl::Const(c) => Some(&c.name.name),
        TopDecl::Interface(i) => Some(&i.name.name),
        _ => None,
    }
}

/// Widen a declaration's range back over the annotations on the lines
/// before it and forward over a trailing `;`.
fn decl_range(src: &str, sp: Span) -> (usize, usize) {
    let mut start = sp.start.as_usize();
    // the line it starts on, from its beginning
    let line_start = src[..start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    if src[line_start..start].trim().starts_with('@') || src[line_start..start].trim().is_empty() {
        start = line_start;
    }
    // earlier lines that are annotations
    loop {
        if start == 0 {
            break;
        }
        let prev_end = start - 1;
        let prev_start = src[..prev_end].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line = src[prev_start..prev_end].trim();
        if line.starts_with('@') {
            start = prev_start;
        } else {
            break;
        }
    }
    (start, past_semicolon(src, sp.end.as_usize()))
}

/// A top-level declaration moves to another file of the same seed.
fn seed_split(files: &Files, rng: &mut Rng) -> Option<(Files, String)> {
    let (src, program) = parsed(files)?;
    if files.contains_key(SPLIT) {
        return None;
    }
    let sites: Vec<&TopDecl> = program.items.iter().filter(|i| decl_span(i).is_some()).collect();
    if sites.len() < 2 {
        return None;
    }
    let item = *rng.pick(&sites)?;
    let (s, e) = decl_range(&src, decl_span(item)?);
    let moved = src.get(s..e)?.to_string();
    let rest = splice(&src, vec![(s, e, String::new())])?;
    let mut out = with_main(files, rest);
    out.insert(SPLIT.to_string(), format!("{}\n", moved.trim()));
    Some((out, format!("seed_split({})", decl_name(item).unwrap_or("?"))))
}

/// `name` qualified as `alias::name` wherever the main file names it
/// bare (not after `.` or `::`, and not as a field or argument label —
/// an identifier followed by `:` in a record literal).
fn requalify(src: &str, name: &str, alias: &str) -> Option<String> {
    let tokens = lex(src).ok()?;
    let mut edits = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        let TokenKind::Ident(n) = &t.kind else { continue };
        if n != name {
            continue;
        }
        let prev = i.checked_sub(1).map(|j| &tokens[j].kind);
        if matches!(prev, Some(TokenKind::Dot) | Some(TokenKind::ColonColon)) {
            continue;
        }
        let next = tokens.get(i + 1).map(|t| &t.kind);
        if matches!(next, Some(TokenKind::Colon)) {
            continue;
        }
        edits.push((t.span.start.as_usize(), t.span.end.as_usize(), format!("{alias}::{name}")));
    }
    if edits.is_empty() {
        return None;
    }
    splice(src, edits)
}

/// A top-level fn, type or const moves to a seed of its own, imported
/// under an alias; the main seed names it through the alias — or, for
/// about half the names (GH #1951 item 6), lists it on the import and
/// keeps naming it bare. The form is a hash of the name rather than a
/// draw, so the RNG stream every other candidate sees is unchanged.
fn import_alias(files: &Files, rng: &mut Rng) -> Option<(Files, String)> {
    let (src, program) = parsed(files)?;
    if files.contains_key(LIB) || !program.imports.is_empty() {
        return None;
    }
    let sites: Vec<&TopDecl> = program
        .items
        .iter()
        .filter(|i| match i {
            TopDecl::Fn(f) => f.name.name != "main" && f.ffi.is_none(),
            TopDecl::Type(_) | TopDecl::Const(_) => true,
            _ => false,
        })
        .collect();
    let item = *rng.pick(&sites)?;
    let name = decl_name(item)?.to_string();
    let (s, e) = decl_range(&src, decl_span(item)?);
    let moved = src.get(s..e)?.to_string();
    let rest = splice(&src, vec![(s, e, String::new())])?;
    // a site either way: the main file names it
    let qualified = requalify(&rest, &name, ALIAS)?;
    if fnv(&name) % 2 == 1 {
        let mut out = with_main(files, format!("import \"./pglib\" as {ALIAS} {{ {name} }};\n\n{rest}"));
        out.insert(LIB.to_string(), format!("{}\n", moved.trim()));
        return Some((out, format!("import_alias({name}, listed)")));
    }
    let rest = qualified;
    let mut out = with_main(files, format!("import \"./pglib\" as {ALIAS};\n\n{rest}"));
    out.insert(LIB.to_string(), format!("{}\n", moved.trim()));
    Some((out, format!("import_alias({name})")))
}

/// A value of one element type for the probes, as an expression.
struct Elem {
    ty: String,
    value: String,
    /// The extra declarations the type needs.
    decls: Vec<String>,
}

/// The element types a probe can carry: the primitives the corpus
/// steers around least (Decimal, U256, String), a payload enum, and the
/// program's own records and enums, valued from a literal the program
/// writes.
fn elem_types(src: &str, program: &Program) -> Vec<Elem> {
    let mut out = vec![
        Elem { ty: "Int".into(), value: "7".into(), decls: vec![] },
        Elem { ty: "String".into(), value: "\"pg\"".into(), decls: vec![] },
        Elem { ty: "Bool".into(), value: "true".into(), decls: vec![] },
        Elem { ty: "Float".into(), value: "1.5".into(), decls: vec![] },
        Elem { ty: "Decimal".into(), value: "2.50d".into(), decls: vec![] },
        Elem { ty: "Decimal<2>".into(), value: "2.50d".into(), decls: vec![] },
        Elem { ty: "U256".into(), value: "U256::from_int(7)".into(), decls: vec![] },
        Elem { ty: "I256".into(), value: "I256::from_int(-7)".into(), decls: vec![] },
        Elem {
            ty: "__PgPay".into(),
            value: "__PgPay::Two(3, \"x\")".into(),
            decls: vec!["type __PgPay = enum { One(Int), Two(Int, String), Zero };".into()],
        },
        Elem {
            ty: "__PgRec".into(),
            value: "__PgRec { a: 1, s: \"r\", d: 1.25d }".into(),
            decls: vec!["type __PgRec {\n    a: Int;\n    s: String;\n    d: Decimal;\n}".into()],
        },
    ];
    // The program's records, valued by a literal it writes with no
    // local in it (a literal of constants alone).
    let declared: Vec<&TypeDecl> = program
        .items
        .iter()
        .filter_map(|i| if let TopDecl::Type(t) = i { Some(t) } else { None })
        .filter(|t| t.generics.is_empty())
        .collect();
    for site in exprs(program) {
        if let Expr::Struct { path, inits, .. } = site.e {
            if path.segments.len() != 1 {
                continue;
            }
            let n = &path.segments[0].name;
            if !declared.iter().any(|t| &t.name.name == n && matches!(t.body, TypeDeclBody::Struct(_))) {
                continue;
            }
            if inits.iter().all(|i| matches!(i.value, Expr::Literal(..))) {
                if let Some(v) = text(src, site.e.span()) {
                    out.push(Elem { ty: n.clone(), value: v.to_string(), decls: vec![] });
                }
            }
        }
    }
    for t in &declared {
        if let TypeDeclBody::Enum(vs) = &t.body {
            if let Some(v) = vs.iter().find(|v| v.fields.is_empty() && !v.member) {
                out.push(Elem { ty: t.name.name.clone(), value: format!("{}::{}", t.name.name, v.name.name), decls: vec![] });
            }
        }
    }
    out
}

/// A probe fn exercising one collection or payload shape over one
/// element type, called from `fn main`: the shapes the issue names
/// (bounded and array fields, a `@form(vec)` and `@form(hashmap)`, an
/// enum payload, a generic record, a fn value) over the types the
/// program uses.
fn element_probe(files: &Files, rng: &mut Rng) -> Option<(Files, String)> {
    let (src, program) = parsed(files)?;
    let main = program.items.iter().find_map(|i| match i {
        TopDecl::Fn(f) if f.name.name == "main" && f.params.is_empty() => Some(f),
        _ => None,
    })?;
    let types = elem_types(&src, &program);
    let el = rng.pick(&types)?;
    let (t, v) = (&el.ty, &el.value);
    let shapes = ["bounded_field", "array", "enum_payload", "generic_record", "fn_value", "vec_form", "map_form", "bounded_param"];
    let shape = *rng.pick(&shapes)?;
    let mut decls = el.decls.clone();
    let body = match shape {
        "bounded_field" => {
            decls.push(format!("type __PgBag {{\n    items: bounded[{t}; 4];\n    n: Int;\n}}"));
            format!(
                "let mut b = __PgBag {{ n: 0 }};\n    push(b.items, {v}) or discard;\n    push(b.items, {v}) or discard;\n    let first = at(b.items, 0) or {v};\n    for x in b.items {{\n        b.n = b.n + 1;\n    }}\n    println(count(b.items), \" \", b.n);"
            )
        }
        "bounded_param" => {
            decls.push(format!("type __PgBag {{\n    items: bounded[{t}; 4];\n}}"));
            decls.push(format!(
                "fn __pg_fill(b: __PgBag, x: {t}) -> __PgBag {{\n    let mut c = b;\n    push(c.items, x) or discard;\n    return c;\n}}"
            ));
            format!("let b = __pg_fill(__PgBag {{ }}, {v});\n    let c = __pg_fill(b, {v});\n    println(count(c.items));")
        }
        "array" => format!(
            "let xs: [{t}; 3] = [{v}, {v}, {v}];\n    let y = xs[1];\n    let mut n = 0;\n    for x in xs {{\n        n = n + 1;\n    }}\n    println(n);"
        ),
        "enum_payload" => {
            decls.push(format!("type __PgOpt = enum {{ Has({t}), Empty }};"));
            decls.push(format!(
                "fn __pg_unwrap(o: __PgOpt, d: {t}) -> {t} {{\n    return match o {{\n        Has(x) -> x,\n        Empty -> d,\n    }};\n}}"
            ));
            format!("let o = __PgOpt::Has({v});\n    let x = __pg_unwrap(o, {v});\n    let e = __pg_unwrap(__PgOpt::Empty, x);")
        }
        "generic_record" => {
            decls.push("type __PgPair<A, B> {\n    a: A;\n    b: B;\n}".into());
            format!(
                "let p = __PgPair {{ a: {v}, b: 1 }};\n    let q = __PgPair {{ a: p.b, b: p.a }};\n    let r = q.b;"
            )
        }
        "fn_value" => {
            decls.push(format!("fn __pg_same(x: {t}) -> {t} {{\n    return x;\n}}"));
            format!("let f: fn({t}) -> {t} = __pg_same;\n    let y = f({v});\n    let z = f(y);")
        }
        "vec_form" => {
            decls.push(format!("@form(vec)\nlocus __PgVec {{\n    capacity {{\n        heap xs of {t};\n    }}\n}}"));
            format!(
                "let v = __PgVec {{ }};\n    v.push({v});\n    v.push({v});\n    let x = v.get(0) or {v};\n    println(v.len());"
            )
        }
        _ => {
            decls.push(format!("type __PgCell {{\n    k: String = \"\";\n    v: {t};\n}}"));
            decls.push("@form(hashmap)\nlocus __PgMap {\n    capacity {\n        pool rows of __PgCell indexed_by k;\n    }\n}".into());
            format!(
                "let m = __PgMap {{ }};\n    m.set(__PgCell {{ k: \"a\", v: {v} }});\n    m.set(__PgCell {{ k: \"b\", v: {v} }});\n    let c = m.get(\"a\") or __PgCell {{ k: \"z\", v: {v} }};\n    println(m.len());"
            )
        }
    };
    let probe = format!("fn __pg_probe() {{\n    {body}\n}}");
    let at = main.body.span.start.as_usize() + 1;
    let mut out = splice(&src, vec![(at, at, "\n    __pg_probe();".to_string())])?;
    for d in &decls {
        out = append(&out, d);
    }
    out = append(&out, &probe);
    Some((with_main(files, out), format!("element_probe({shape}, {t})")))
}

/// Free fns the program declares by name, the ones a call can be routed
/// around (written, not generic).
fn free_fns(program: &Program) -> Vec<&FnDecl> {
    program
        .items
        .iter()
        .filter_map(|i| if let TopDecl::Fn(f) = i { Some(f) } else { None })
        .filter(|f| f.name.name != "main" && f.generics.is_empty() && rewritable(f))
        .collect()
}

/// Calls `f(args)` to one of `fns`, by bare name.
fn calls_to<'a>(program: &'a Program, fns: &[&'a FnDecl]) -> Vec<(ExprSite<'a>, &'a FnDecl)> {
    exprs(program)
        .into_iter()
        .filter_map(|s| match s.e {
            Expr::Call { callee, .. } => match callee.as_ref() {
                Expr::Ident(id) => fns.iter().find(|f| f.name.name == id.name).map(|f| (s, *f)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn sig_parts(src: &str, f: &FnDecl) -> Option<(Vec<(String, String)>, Option<String>, Option<String>)> {
    let mut params = Vec::new();
    for p in &f.params {
        params.push((p.name.name.clone(), text(src, p.ty.span())?.to_string()));
    }
    let ret = match &f.ret {
        Some(r) => Some(text(src, r.span())?.to_string()),
        None => None,
    };
    let fallible = match &f.fallible {
        Some(t) => Some(text(src, t.span())?.to_string()),
        None => None,
    };
    Some((params, ret, fallible))
}

/// Each parameter declared as written, its default included
/// (`n: Int = 2`), for a fn that stands in for `f` at its call sites.
fn defaults(src: &str, f: &FnDecl) -> Option<Vec<String>> {
    let mut out = Vec::new();
    for p in &f.params {
        let ty = text(src, p.ty.span())?;
        out.push(match &p.default {
            Some(d) => format!("{}: {ty} = {}", p.name.name, text(src, d.span())?),
            None => format!("{}: {ty}", p.name.name),
        });
    }
    Some(out)
}

/// A direct call `f(a, b)` goes through a fn value:
/// `let __pg_fv: fn(A, B) -> R = f;` then `__pg_fv(a, b)`.
fn call_via_fn_value(files: &Files, rng: &mut Rng) -> Option<(Files, String)> {
    let (src, program) = parsed(files)?;
    let fns: Vec<&FnDecl> = free_fns(&program).into_iter().filter(|f| f.fallible.is_none()).collect();
    let calls = calls_to(&program, &fns);
    let (site, f) = rng.pick(&calls)?;
    let (params, ret, _) = sig_parts(&src, f)?;
    let tys: Vec<&str> = params.iter().map(|(_, t)| t.as_str()).collect();
    let ty = match &ret {
        Some(r) => format!("fn({}) -> {r}", tys.join(", ")),
        None => format!("fn({})", tys.join(", ")),
    };
    let Expr::Call { callee, .. } = site.e else { return None };
    let cs = callee.span();
    let out = splice(
        &src,
        vec![
            (site.before, site.before, format!("let __pg_fv: {ty} = {};\n    ", f.name.name)),
            (cs.start.as_usize(), cs.end.as_usize(), "__pg_fv".to_string()),
        ],
    )?;
    Some((with_main(files, out), format!("call_via_fn_value({})", f.name.name)))
}

/// A direct call `f(args)` goes through a forwarding fn of the same
/// signature (fallible ones forward the error with `or fail err`).
fn call_via_wrapper(files: &Files, rng: &mut Rng) -> Option<(Files, String)> {
    let (src, program) = parsed(files)?;
    let fns = free_fns(&program);
    let calls = calls_to(&program, &fns);
    let (site, f) = rng.pick(&calls)?;
    let (params, ret, fallible) = sig_parts(&src, f)?;
    let ps: Vec<String> = defaults(&src, f)?;
    let names: Vec<&str> = params.iter().map(|(n, _)| n.as_str()).collect();
    let call = format!("{}({})", f.name.name, names.join(", "));
    let call = if fallible.is_some() { format!("{call} or fail err") } else { call };
    let body = if ret.is_some() { format!("return {call};") } else { format!("{call};") };
    let head = format!(
        "fn __pg_fwd({}){}{}",
        ps.join(", "),
        ret.as_ref().map(|r| format!(" -> {r}")).unwrap_or_default(),
        fallible.as_ref().map(|e| format!(" fallible({e})")).unwrap_or_default()
    );
    let Expr::Call { callee, .. } = site.e else { return None };
    let cs = callee.span();
    let out = splice(&src, vec![(cs.start.as_usize(), cs.end.as_usize(), "__pg_fwd".to_string())])?;
    let out = append(&out, &format!("{head} {{\n    {body}\n}}"));
    Some((with_main(files, out), format!("call_via_wrapper({})", f.name.name)))
}

/// Method calls `recv.m(args)` to a locus member `m` no other locus or
/// type of the program declares.
fn method_calls<'a>(program: &'a Program) -> Vec<(ExprSite<'a>, &'a FnDecl, &'a LocusDecl)> {
    let members: Vec<FnSite> = fns(program).into_iter().filter(|s| s.locus.is_some()).collect();
    exprs(program)
        .into_iter()
        .filter_map(|s| {
            let Expr::Call { callee, .. } = s.e else { return None };
            let Expr::Field { name, .. } = callee.as_ref() else { return None };
            let mut owners = members.iter().filter(|m| m.f.name.name == name.name);
            let one = owners.next()?;
            if owners.next().is_some() {
                return None;
            }
            Some((s, one.f, one.locus?))
        })
        .collect()
}

fn mentions_self(src: &str, b: &Block) -> bool {
    text(src, b.span).is_none_or(|t| t.contains("self"))
}

/// A locus member that never reads `self` is also written as a free fn,
/// and a call `recv.m(args)` becomes `__pg_free_m(args)`.
fn method_to_free(files: &Files, rng: &mut Rng) -> Option<(Files, String)> {
    let (src, program) = parsed(files)?;
    let calls: Vec<_> = method_calls(&program)
        .into_iter()
        .filter(|(_, f, _)| f.generics.is_empty() && rewritable(f) && !mentions_self(&src, &f.body))
        .collect();
    let (site, f, l) = rng.pick(&calls)?;
    let (_, ret, fallible) = sig_parts(&src, f)?;
    let ps: Vec<String> = defaults(&src, f)?;
    let name = format!("__pg_free_{}", f.name.name);
    let head = format!(
        "fn {name}({}){}{}",
        ps.join(", "),
        ret.as_ref().map(|r| format!(" -> {r}")).unwrap_or_default(),
        fallible.as_ref().map(|e| format!(" fallible({e})")).unwrap_or_default()
    );
    let body = text(&src, f.body.span)?;
    let Expr::Call { args, span, .. } = site.e else { return None };
    let mut arg_text = Vec::new();
    for a in args {
        arg_text.push(text(&src, a.span())?.to_string());
    }
    let out = splice(
        &src,
        vec![(span.start.as_usize(), span.end.as_usize(), format!("{name}({})", arg_text.join(", ")))],
    )?;
    let out = append(&out, &format!("{head} {body}"));
    Some((with_main(files, out), format!("method_to_free({}.{})", l.name.name, f.name.name)))
}

/// A call `recv.m(args)` dispatches through an interface: an
/// `interface __PgI { fn m(..) -> R; }` every locus with `m` satisfies,
/// and a free fn taking an `__PgI`.
fn interface_dispatch(files: &Files, rng: &mut Rng) -> Option<(Files, String)> {
    let (src, program) = parsed(files)?;
    let calls: Vec<_> = method_calls(&program).into_iter().filter(|(_, f, _)| f.generics.is_empty()).collect();
    let (site, f, l) = rng.pick(&calls)?;
    let (params, ret, fallible) = sig_parts(&src, f)?;
    let ps: Vec<String> = params.iter().map(|(n, t)| format!("{n}: {t}")).collect();
    let names: Vec<&str> = params.iter().map(|(n, _)| n.as_str()).collect();
    let tail = format!(
        "{}{}",
        ret.as_ref().map(|r| format!(" -> {r}")).unwrap_or_default(),
        fallible.as_ref().map(|e| format!(" fallible({e})")).unwrap_or_default()
    );
    let iface = format!("interface __PgI {{\n    fn {}({}){tail};\n}}", f.name.name, ps.join(", "));
    let mut via_ps = vec!["x: __PgI".to_string()];
    via_ps.extend(ps.iter().cloned());
    let call = format!("x.{}({})", f.name.name, names.join(", "));
    let call = if fallible.is_some() { format!("{call} or fail err") } else { call };
    let body = if ret.is_some() { format!("return {call};") } else { format!("{call};") };
    let via = format!("fn __pg_via({}){tail} {{\n    {body}\n}}", via_ps.join(", "));
    let Expr::Call { callee, args, span, .. } = site.e else { return None };
    let Expr::Field { receiver, .. } = callee.as_ref() else { return None };
    let mut arg_text = vec![text(&src, receiver.span())?.to_string()];
    for a in args {
        arg_text.push(text(&src, a.span())?.to_string());
    }
    let out = splice(
        &src,
        vec![(span.start.as_usize(), span.end.as_usize(), format!("__pg_via({})", arg_text.join(", ")))],
    )?;
    let out = append(&append(&out, &iface), &via);
    Some((with_main(files, out), format!("interface_dispatch({}.{})", l.name.name, f.name.name)))
}

/// An `or` clause takes another disposition: `or raise`, `or discard`,
/// `or fail err`, or a substitute value.
fn or_vary(files: &Files, rng: &mut Rng) -> Option<(Files, String)> {
    let (src, program) = parsed(files)?;
    let sites: Vec<ExprSite> = exprs(&program).into_iter().filter(|s| matches!(s.e, Expr::Or { .. })).collect();
    let site = rng.pick(&sites)?;
    let Expr::Or { inner, span, .. } = site.e else { return None };
    let mut options = vec!["raise", "discard", "0", "\"\"", "false", "0.0", "0.0d", "[]"];
    if site.fallible {
        options.push("fail err");
    }
    let pick = *rng.pick(&options)?;
    let out = splice(&src, vec![(inner.span().end.as_usize(), span.end.as_usize(), format!(" or {pick}"))])?;
    Some((with_main(files, out), format!("or_vary({} -> or {pick})", describe(&src, site.e))))
}

/// A call `f(args)` to a non-fallible free fn goes through a fallible
/// wrapper (`fallible(String)`, with a failure path that never runs),
/// addressed at the call by `or raise`, `or discard`, `or fail err`, or a
/// substitute: the error channel plumbed through code that had none.
fn fallible_wrap(files: &Files, rng: &mut Rng) -> Option<(Files, String)> {
    let (src, program) = parsed(files)?;
    let fns: Vec<&FnDecl> = free_fns(&program).into_iter().filter(|f| f.fallible.is_none()).collect();
    let calls = calls_to(&program, &fns);
    let (site, f) = rng.pick(&calls)?;
    let (params, ret, _) = sig_parts(&src, f)?;
    let ps: Vec<String> = defaults(&src, f)?;
    let names: Vec<&str> = params.iter().map(|(n, _)| n.as_str()).collect();
    let call = format!("{}({})", f.name.name, names.join(", "));
    let body = if ret.is_some() { format!("return {call};") } else { format!("{call};") };
    let head = format!(
        "fn __pg_try({}){} fallible(String)",
        ps.join(", "),
        ret.as_ref().map(|r| format!(" -> {r}")).unwrap_or_default()
    );
    let wrapper = format!("{head} {{\n    if __PG_NEVER {{\n        fail \"never\";\n    }}\n    {body}\n}}");
    let dispositions: Vec<String> = match &ret {
        Some(_) => {
            let Expr::Call { .. } = site.e else { return None };
            // a substitute of the call's own value, evaluated only on failure
            let again = text(&src, site.e.span())?.to_string();
            vec!["raise".into(), again]
        }
        None => vec!["raise".into(), "discard".into()],
    };
    let mut dispositions = dispositions;
    if site.fallible {
        dispositions.push("fail err".into());
    }
    let d = rng.pick(&dispositions)?.clone();
    let Expr::Call { callee, span, .. } = site.e else { return None };
    let cs = callee.span();
    let call_text = text(&src, *span)?;
    let rel = cs.end.as_usize() - span.start.as_usize();
    let new_call = format!("(__pg_try{} or {d})", &call_text[rel..]);
    let out = splice(&src, vec![(span.start.as_usize(), span.end.as_usize(), new_call)])?;
    let out = append(&out, "const __PG_NEVER: Bool = false;");
    let out = append(&out, &wrapper);
    let shown = if d.len() > 20 { "substitute" } else { d.as_str() };
    Some((with_main(files, out), format!("fallible_wrap({}, or {shown})", f.name.name)))
}
