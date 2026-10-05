//! F.40 phase 4, line S, step S0: every stdlib dispatch arm is reached
//! by the shadow set at every position it is dispatched from.
//!
//! **Scaffolding.** Line S moves the stdlib call paths out of the three
//! hand-written dispatchers into one table-driven `match`, with IR
//! identity over the shadow set as the only proof that nothing changed.
//! That proof is only as good as the set's reach, so this file holds it
//! to every arm before an arm moves. Since S3 the statement and value
//! positions dispatch from the row (`lower_std_call`): an intrinsic's id
//! picks its arm in `lower_std_intrinsic`'s `match id`, whose arms and
//! position branches this file scrapes ([`id_arms`]), a Hale-body row is
//! lowered by `lower_std_hale_body` (its two lists scraped), and the
//! bare refusal is the arm that calls `bare_call_of_a_fallible_row`.
//! Since S4 the `or` position dispatches from the row too
//! (`lower_std_fallible_call`): an id picks its arm in
//! `lower_std_intrinsic_fallible`'s `match id` ([`fallible_id_arms`]),
//! and its refusal is the arm that calls `or_over_an_infallible_row`.
//! Since S5 the two refusals are the rows' fallibility, not id lists
//! ([`refused_by_the_rows`]). No position matches a `["std", ..]` literal
//! any more. `stdlib_registry_parity` reads the same scrape, with what
//! each arm calls ([`ArmCall`]).
//!
//! A *pair* is a (path, position): a stdlib call path an arm lowers or
//! refuses, and the position it does so at. The *shadow set* is what
//! the line's IR comparison builds: the corpus examples, the lifecycle
//! fixtures, `tests/hale`, the DNA mains, every single-file program
//! `hale_corpus::embedded` harvests out of the Rust tests that checks
//! clean, and the call fixtures under `fixtures/stdlib_calls/`. A pair
//! is covered when some program of the set CALLS that path AT that
//! position, found by walking its syntax tree (`CallWalk`) and
//! classifying each call the way lowering routes it. The pairs no
//! checked program can reach are allowances, each kind with its reason
//! and each list held to the code (`the_allowances_are_what_the_code_says`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use hale_syntax::ast::*;

#[path = "support/harness.rs"]
mod harness;

// ---------------------------------------------------------------------
// Positions, and how lowering decides them.
// ---------------------------------------------------------------------

/// Which position a `std::` path call is dispatched at. The rule,
/// mirrored by `CallWalk` below, lives in
/// `crates/hale-codegen/src/codegen.rs`:
///
/// * `lower_stmt_at`'s `Stmt::Expr(Expr::Call { callee: Expr::Path .. })`
///   arm calls `lower_path_call`, which sends a `std` path to
///   `lower_stdlib_path_call`, `lower_std_call` with
///   `StdCallPos::Statement`: a call that IS a
///   statement. A statement-position `match` whose arm body is a call
///   expression routes that body through `lower_stmt` too
///   (`lower_match_core`, `capture: None`), so it is a statement as
///   well. An arm that does not match on `pos` is the value position's,
///   its value dropped by a statement, so such a statement is lowered by
///   an EXPRESSION pair: [`lowered_at`] says which.
/// * `lower_or_expr` (`channels/mod.rs`, reached from the `Stmt::Expr(
///   Expr::Or ..)` statement and from `lower_expr`'s `Expr::Or`) calls
///   `lower_fallible_call`, whose `Expr::Path` callee goes to
///   `try_lower_fallible_stdlib_path_call`, `lower_std_fallible_call`:
///   the call directly under an `or`, wherever the `or` stands.
/// * Every other call — an argument, a `let` value, a block's tail
///   (`lower_block` and `lower_block_as_expr` both lower the tail with
///   `lower_expr`), a value-producing `match` arm — goes through
///   `lower_expr`'s `Expr::Call` arm to `lower_path_call_expr` and
///   `lower_stdlib_path_call_expr`, `lower_std_call` with
///   `StdCallPos::Value` (the EXPRESSION position here).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Position {
    Statement,
    Expression,
    Fallible,
}

impl Position {
    pub fn word(self) -> &'static str {
        match self {
            Position::Statement => "statement",
            Position::Expression => "expression",
            Position::Fallible => "fallible",
        }
    }
}

/// The position whose arm lowers a call the walk found at `position`: a
/// statement call of a path with no statement branch is the value
/// position's arm, its value dropped.
pub fn lowered_at(path: &str, position: Position, statement_paths: &BTreeMap<String, (ArmKind, usize)>) -> Position {
    if position == Position::Statement && !statement_paths.contains_key(path) {
        Position::Expression
    } else {
        position
    }
}

// ---------------------------------------------------------------------
// The scrape: Rust source → arms → pairs.
// ---------------------------------------------------------------------

fn crate_file(rel: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

/// `src` with comments blanked to spaces, and string and char literal
/// contents too when `strings` is set. Byte offsets and newlines are
/// kept, so a position found in one mask indexes the source.
fn mask(src: &str, strings: bool) -> String {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for c in &mut out[from..to] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let end = b[i..].iter().position(|&c| c == b'\n').map_or(b.len(), |n| i + n);
                blank(&mut out, i, end);
                i = end;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let (mut depth, mut j) = (1, i + 2);
                while j < b.len() && depth > 0 {
                    if b[j] == b'/' && b.get(j + 1) == Some(&b'*') {
                        depth += 1;
                        j += 2;
                    } else if b[j] == b'*' && b.get(j + 1) == Some(&b'/') {
                        depth -= 1;
                        j += 2;
                    } else {
                        j += 1;
                    }
                }
                blank(&mut out, i, j);
                i = j;
            }
            b'r' if (i == 0 || !ident(b[i - 1]))
                && matches!(b.get(i + 1), Some(b'"') | Some(b'#')) =>
            {
                let mut j = i + 1;
                while b.get(j) == Some(&b'#') {
                    j += 1;
                }
                if b.get(j) != Some(&b'"') {
                    i += 1;
                    continue;
                }
                let hashes = j - i - 1;
                let close: Vec<u8> =
                    std::iter::once(b'"').chain(std::iter::repeat(b'#').take(hashes)).collect();
                let body = j + 1;
                let end = b[body..]
                    .windows(close.len())
                    .position(|w| w == close.as_slice())
                    .map_or(b.len(), |n| body + n);
                if strings {
                    blank(&mut out, body, end);
                }
                i = end + close.len();
            }
            b'"' => {
                let mut j = i + 1;
                while j < b.len() && b[j] != b'"' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                if strings {
                    blank(&mut out, i + 1, j.min(b.len()));
                }
                i = j + 1;
            }
            b'\'' => {
                // A char literal ('x', '\n', '\u{..}', a multi-byte char)
                // or a lifetime ('ctx), which has no closing quote.
                let close = if b.get(i + 1) == Some(&b'\\') {
                    b[i + 2..].iter().position(|&c| c == b'\'').map(|n| i + 2 + n)
                } else {
                    let ch_len = src[i + 1..].chars().next().map_or(1, char::len_utf8);
                    (b.get(i + 1 + ch_len) == Some(&b'\'')).then_some(i + 1 + ch_len)
                };
                match close {
                    Some(c) => {
                        if strings {
                            blank(&mut out, i + 1, c);
                        }
                        i = c + 1;
                    }
                    None => i += 1,
                }
            }
            _ => i += 1,
        }
    }
    String::from_utf8(out).expect("masking replaces whole literals")
}

/// The byte range of `fn name`'s body (from its `{` to its `}`) in a
/// source whose comments and literals `code` has masked.
fn fn_body(code: &str, name: &str) -> (usize, usize) {
    let needle = format!("fn {name}(");
    let start = code
        .match_indices(&needle)
        .map(|(i, _)| i)
        .find(|&i| i == 0 || !code.as_bytes()[i - 1].is_ascii_alphanumeric())
        .unwrap_or_else(|| panic!("no `fn {name}` in the dispatchers' source"));
    let open = start + code[start..].find('{').expect("a body");
    let mut depth = 0usize;
    for (k, c) in code[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return (open, open + k);
                }
            }
            _ => {}
        }
    }
    panic!("unbalanced body of `fn {name}`");
}

/// One arm of the row dispatch at one position: the paths of the ids
/// (or Hale bodies) it names, or of a refusal list.
#[derive(Clone, Debug)]
pub struct Arm {
    /// 1-indexed line of the arm in `codegen.rs`.
    pub line: usize,
    paths: Vec<String>,
    /// One of the refusal lists.
    pub refuses: bool,
    /// What the arm's body calls to lower the path.
    pub calls: ArmCall,
}

/// How an arm lowers its path: a Hale body of the stdlib seeds, called
/// by the name its row gives (`lower_std_hale_body`), or natively (an
/// intrinsic's arm).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArmCall {
    HaleBody(String),
    Native,
}

fn string_literals(s: &str) -> Vec<String> {
    s.split('"').skip(1).step_by(2).map(str::to_string).collect()
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn line_indent(text: &str, at: usize) -> usize {
    let start = text[..at].rfind('\n').map_or(0, |n| n + 1);
    indent_of(&text[start..])
}

/// What an arm does with a path: lower it, or refuse it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmKind {
    Lowers,
    Refuses,
}

/// One position's arms, scraped.
pub struct Scraped {
    pub position: Position,
    pub arms: Vec<Arm>,
    /// Every path an arm names, with the kind of the FIRST arm naming it
    /// (the one that answers) and its line.
    pub paths: BTreeMap<String, (ArmKind, usize)>,
    /// What that arm calls, for each path.
    pub calls: BTreeMap<String, ArmCall>,
    /// Paths an earlier arm already named: an arm no call reaches.
    pub shadowed: Vec<(String, usize)>,
}

// ---------------------------------------------------------------------
// The row dispatch (S3, S4): `lower_std_call` looks the path's row up,
// and an intrinsic's id picks its arm in `lower_std_intrinsic`'s
// `match id` while a Hale body is called by the name the row gives
// (`lower_std_hale_body`); under `or`, `lower_std_fallible_call` looks
// it up and the id picks its arm in `lower_std_intrinsic_fallible`'s.
// Their arms are scraped into arms of the three positions, as the
// legacy dispatchers' were.
// ---------------------------------------------------------------------

/// What an arm of `lower_std_intrinsic` does at one position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Branch {
    /// It lowers the call.
    Lowers,
    /// It answers as a path no arm lowers (`lower_std_unarmed`).
    Unarmed,
    /// It refuses a bare call of a function whose row is fallible
    /// (`bare_call_of_a_fallible_row`).
    Refuses,
}

fn branch(text: &str) -> Branch {
    if text.contains("bare_call_of_a_fallible_row(") {
        Branch::Refuses
    } else if text.contains("lower_std_unarmed(") {
        Branch::Unarmed
    } else {
        Branch::Lowers
    }
}

/// One arm of `lower_std_intrinsic`'s `match id`.
pub struct IdArm {
    pub line: usize,
    pub ids: Vec<String>,
    /// The statement branch, when the arm matches on `pos`; `None` when
    /// a statement drops the value position's answer.
    pub statement: Option<Branch>,
    pub value: Branch,
}

/// The arms of `lower_std_intrinsic`'s `match id`, with their positions'
/// branches.
pub fn id_arms() -> Vec<IdArm> {
    match_id_arms("lower_std_intrinsic")
        .into_iter()
        .map(|(line, ids, text)| {
            let (statement, value) = if text.contains("match pos") {
                let s = text.find("StdCallPos::Statement =>").expect("a statement branch");
                let v = text.find("StdCallPos::Value =>").expect("a value branch");
                assert!(s < v, "line {line}: the statement branch comes first");
                (Some(branch(&text[s..v])), branch(&text[v..]))
            } else {
                (None, branch(&text))
            };
            IdArm { line, ids, statement, value }
        })
        .collect()
}

/// The arms of `lower_std_intrinsic_fallible`'s `match id`, each with
/// whether it lowers the call under `or`: the arm that refuses an `or`
/// over a function whose row cannot fail (`or_over_an_infallible_row`)
/// lowers nothing.
pub fn fallible_id_arms() -> Vec<(usize, Vec<String>, bool)> {
    match_id_arms("lower_std_intrinsic_fallible")
        .into_iter()
        .map(|(line, ids, text)| (line, ids, !text.contains("or_over_an_infallible_row(")))
        .collect()
}

/// The arms of `fn func`'s `match id` in `codegen.rs`, in source order:
/// each arm's line, its ids and its body's text. The match has no `_`
/// arm: every id is named by exactly one arm.
fn match_id_arms(func: &str) -> Vec<(usize, Vec<String>, String)> {
    let src = crate_file("src/codegen.rs");
    let no_comments = mask(&src, false);
    let code = mask(&src, true);
    let (open, close) = fn_body(&code, func);
    let body = &no_comments[open..close];
    let first_line = src[..open].matches('\n').count() + 1;
    let m = body.find("match id {").unwrap_or_else(|| panic!("`{func}` matches on `id`"));
    let arm_indent = line_indent(body, m) + 4;
    let lines: Vec<&str> = body.lines().collect();
    let ends_match = |l: &str| indent_of(l) < arm_indent && !l.trim().is_empty();
    let is_head = |l: &str| indent_of(l) == arm_indent && l.trim_start().starts_with("Id::");
    let mut arms = Vec::new();
    let mut i = body[..m].matches('\n').count() + 1;
    while i < lines.len() && !ends_match(lines[i]) {
        let l = lines[i];
        if indent_of(l) == arm_indent && l.trim_start().starts_with('_') {
            panic!("`{func}` has a `_` arm (line {}): an id without an arm would compile", first_line + i);
        }
        if !is_head(l) {
            i += 1;
            continue;
        }
        let start = i;
        let mut head = String::new();
        while !lines[i].contains("=>") {
            head.push_str(lines[i]);
            head.push(' ');
            i += 1;
        }
        let k = lines[i].find("=>").unwrap();
        head.push_str(&lines[i][..k]);
        let end = (i + 1..lines.len()).find(|&j| is_head(lines[j]) || ends_match(lines[j])).unwrap_or(lines.len());
        let mut text = lines[i][k + 2..].to_string();
        for l in &lines[i + 1..end] {
            text.push('\n');
            text.push_str(l);
        }
        let ids = head
            .split('|')
            .map(|p| p.trim().strip_prefix("Id::").unwrap_or_else(|| panic!("not an id pattern: {p}")).to_string())
            .collect();
        arms.push((first_line + start, ids, text));
        i = end;
    }
    arms
}

/// Each intrinsic's id, by its name, with its path.
fn intrinsic_paths() -> BTreeMap<String, String> {
    hale_types::stdlib_surface::rows()
        .filter_map(|(s, f)| match f.lower {
            hale_types::stdlib_surface::Lower::Intrinsic(id) => {
                Some((format!("{id:?}"), format!("std::{}::{}", s.ns.join("::"), f.name)))
            }
            _ => None,
        })
        .collect()
}

/// The line `fn func` is on, in a source and its masked copy.
fn fn_line(src: &str, code: &str, func: &str) -> usize {
    let (open, _) = fn_body(code, func);
    src[..src[..open].rfind(&format!("fn {func}(")).expect("the fn")].matches('\n').count() + 1
}

/// The string list `const name: &[&str]` in `fn func`'s body, and the
/// line `fn func` is on.
fn const_list_in(func: &str, name: &str) -> (Vec<String>, usize) {
    let src = crate_file("src/codegen.rs");
    let no_comments = mask(&src, false);
    let code = mask(&src, true);
    let (open, close) = fn_body(&code, func);
    let body = &no_comments[open..close];
    let at = body.find(&format!("const {name}: &[&str]")).unwrap_or_else(|| panic!("no `{name}` in `{func}`"));
    let end = at + body[at..].find("];").expect("the list closes");
    (string_literals(&body[at..end]), fn_line(&src, &code, func))
}

/// The row dispatch's arms at the statement, expression and fallible
/// positions.
struct RowDispatch {
    statement: Vec<Arm>,
    expression: Vec<Arm>,
    fallible: Vec<Arm>,
}

fn row_dispatch() -> RowDispatch {
    let paths = intrinsic_paths();
    let mut rd = RowDispatch { statement: Vec::new(), expression: Vec::new(), fallible: Vec::new() };
    let mut named = BTreeSet::new();
    let arm = |line: usize, paths: &[&String], refuses: bool, calls: ArmCall| Arm {
        line,
        paths: paths.iter().map(|p| p.to_string()).collect(),
        refuses,
        calls,
    };
    let path_of = |id: &String| paths.get(id).unwrap_or_else(|| panic!("`Id::{id}` has no row"));
    // A bare refusal (S5) is the value position's arm, which a statement
    // reaches with its value dropped: an expression pair, as every arm
    // with no statement branch is.
    for a in id_arms() {
        for id in &a.ids {
            assert!(named.insert(id.clone()), "`Id::{id}` is named by two arms of `lower_std_intrinsic`");
        }
        let ps: Vec<&String> = a.ids.iter().map(path_of).collect();
        match a.value {
            Branch::Lowers => rd.expression.push(arm(a.line, &ps, false, ArmCall::Native)),
            Branch::Refuses => rd.expression.push(arm(a.line, &ps, true, ArmCall::Native)),
            Branch::Unarmed => {}
        }
        if a.statement == Some(Branch::Lowers) {
            rd.statement.push(arm(a.line, &ps, false, ArmCall::Native));
        }
    }
    let unnamed: Vec<&String> = paths.keys().filter(|id| !named.contains(*id)).collect();
    assert!(unnamed.is_empty(), "ids no arm of `lower_std_intrinsic` names: {unnamed:?}");
    // The `or` position (S4): an arm that lowers is a pair, and so is the
    // arm that refuses an `or` over a function that cannot fail (S5).
    let mut named = BTreeSet::new();
    for (line, ids, lowers) in fallible_id_arms() {
        for id in &ids {
            assert!(named.insert(id.clone()), "`Id::{id}` is named by two arms of `lower_std_intrinsic_fallible`");
        }
        let ps: Vec<&String> = ids.iter().map(path_of).collect();
        rd.fallible.push(arm(line, &ps, !lowers, ArmCall::Native));
    }
    let unnamed: Vec<&String> = paths.keys().filter(|id| !named.contains(*id)).collect();
    assert!(unnamed.is_empty(), "ids no arm of `lower_std_intrinsic_fallible` names: {unnamed:?}");
    let (statement_bodies, line) = const_list_in("lower_std_hale_body", "STATEMENT_BODIES");
    let (no_value_bodies, _) = const_list_in("lower_std_hale_body", "NO_VALUE_BODIES");
    for (s, f) in hale_types::stdlib_surface::rows() {
        let hale_types::stdlib_surface::Lower::HaleBody(body) = f.lower else { continue };
        let path = format!("std::{}::{}", s.ns.join("::"), f.name);
        let call = ArmCall::HaleBody(body.to_string());
        if !no_value_bodies.iter().any(|b| b == body) {
            rd.expression.push(arm(line, &[&path], false, call.clone()));
        }
        if statement_bodies.iter().any(|b| b == body) {
            rd.statement.push(arm(line, &[&path], false, call));
        }
    }
    rd
}

pub fn scrape() -> Vec<Scraped> {
    let rd = row_dispatch();
    [Position::Statement, Position::Expression, Position::Fallible]
        .into_iter()
        .map(|position| {
            let arms = match position {
                Position::Statement => rd.statement.clone(),
                Position::Expression => rd.expression.clone(),
                Position::Fallible => rd.fallible.clone(),
            };
            let mut paths = BTreeMap::new();
            let mut calls = BTreeMap::new();
            let mut shadowed = Vec::new();
            for arm in &arms {
                let kind = if arm.refuses { ArmKind::Refuses } else { ArmKind::Lowers };
                for path in &arm.paths {
                    if paths.contains_key(path) {
                        shadowed.push((path.clone(), arm.line));
                    } else {
                        calls.insert(path.clone(), arm.calls.clone());
                        paths.insert(path.clone(), (kind, arm.line));
                    }
                }
            }
            Scraped { position, arms, paths, calls, shadowed }
        })
        .collect()
}

/// Every (path, position) pair, with what its arm does.
pub fn pairs(scraped: &[Scraped]) -> BTreeMap<(String, Position), (ArmKind, usize)> {
    let mut out = BTreeMap::new();
    for s in scraped {
        for (path, kind) in &s.paths {
            out.insert((path.clone(), s.position), *kind);
        }
    }
    out
}

/// The statement position as scraped.
fn statement_scrape(scraped: &[Scraped]) -> &Scraped {
    scraped.iter().find(|s| s.position == Position::Statement).expect("a statement position")
}

/// The pair one walked call covers: the arm that lowers it.
fn covers(path: String, position: Position, statement: &Scraped) -> Vec<(String, Position)> {
    let at = lowered_at(&path, position, &statement.paths);
    vec![(path, at)]
}

// ---------------------------------------------------------------------
// The walk: a program → its std calls, by position.
// ---------------------------------------------------------------------

/// A walk over every expression of a program, written after
/// `hale_syntax::sites`' (the one traversal that reaches every
/// `Expr::Call`), recording each `std::` path call with the position
/// lowering dispatches it from (see [`Position`]) and the declaration
/// it sits in. `calls` counts every call it met, so a program's count
/// can be held to the site walk's: a variant this walk skipped would
/// make them differ.
#[derive(Default)]
pub struct CallWalk {
    pub found: Vec<(String, Position, String)>,
    pub calls: usize,
    owner: String,
}

impl CallWalk {
    pub fn program(p: &Program) -> CallWalk {
        let mut w = CallWalk::default();
        w.items(&p.items);
        w
    }

    fn items(&mut self, items: &[TopDecl]) {
        for item in items {
            self.top_decl(item);
        }
    }

    fn top_decl(&mut self, d: &TopDecl) {
        match d {
            TopDecl::Locus(l) => self.locus(l),
            TopDecl::Perspective(p) => {
                self.owner = p.name.name.clone();
                self.generics(&p.generics);
                for member in &p.members {
                    match member {
                        PerspectiveMember::Params(pb) => self.params_block(pb),
                        PerspectiveMember::StableWhen(b) => self.block(b),
                        PerspectiveMember::SerializeAs(t) => self.ty(t),
                        PerspectiveMember::Fn(fd) => {
                            self.owner = format!("{}.{}", p.name.name, fd.name.name);
                            self.fn_decl(fd);
                        }
                        PerspectiveMember::Bus(_) => {}
                    }
                }
            }
            TopDecl::Type(t) => self.type_decl(t),
            TopDecl::Const(c) => {
                self.owner = c.name.name.clone();
                self.ty(&c.ty);
                self.expr(&c.value);
            }
            TopDecl::Fn(fd) => {
                self.owner = fd.name.name.clone();
                self.fn_decl(fd);
            }
            TopDecl::Module(md) => self.items(&md.items),
            TopDecl::Interface(i) => {
                for sig in &i.methods {
                    self.params(&sig.params);
                    self.opt_ty(&sig.ret);
                    self.opt_ty(&sig.fallible);
                }
            }
            TopDecl::Topic(t) => self.ty(&t.payload),
            TopDecl::Group(_)
            | TopDecl::RingLayout(_)
            | TopDecl::Target(_)
            | TopDecl::Role(_)
            | TopDecl::Claims(_)
            | TopDecl::Constitution(_) => {}
        }
    }

    fn locus(&mut self, l: &LocusDecl) {
        self.owner = l.name.name.clone();
        self.generics(&l.generics);
        if let Some(form) = &l.form {
            for arg in &form.args {
                self.expr(&arg.value);
            }
        }
        for member in &l.members {
            self.owner = match member {
                LocusMember::Fn(fd) => format!("{}.{}", l.name.name, fd.name.name),
                _ => l.name.name.clone(),
            };
            self.locus_member(member);
        }
    }

    fn locus_member(&mut self, member: &LocusMember) {
        match member {
            LocusMember::Params(pb) => self.params_block(pb),
            LocusMember::Contract(cb) => match &cb.kind {
                ContractKind::Inferred => {}
                ContractKind::Members(ms) => {
                    for cm in ms {
                        self.opt_ty(&cm.ty);
                    }
                }
            },
            LocusMember::Bus(bb) => {
                for m in &bb.members {
                    match m {
                        BusMember::Subscribe { ty, key_filter, .. } => {
                            self.opt_ty(ty);
                            if let Some(KeyFilter::Specific { expr, .. }) = key_filter {
                                self.expr(expr);
                            }
                        }
                        BusMember::Publish { ty, .. } => self.opt_ty(ty),
                    }
                }
            }
            LocusMember::Lifecycle(ld) => {
                self.params(&ld.params);
                self.opt_ty(&ld.ret);
                self.block(&ld.body);
            }
            LocusMember::Mode(md) => {
                self.params(&md.params);
                self.opt_ty(&md.ret);
                self.block(&md.body);
            }
            LocusMember::Failure(fd) => {
                self.params(&fd.params);
                self.block(&fd.body);
            }
            LocusMember::Closure(cd) => {
                if let Some(a) = &cd.assertion {
                    self.expr(&a.left);
                    self.expr(&a.right);
                    self.expr(&a.tolerance);
                }
                for clause in &cd.clauses {
                    if let ClosureClause::Epoch(EpochSpec::Duration(e)) = clause {
                        self.expr(e);
                    }
                }
            }
            LocusMember::Fn(fd) => self.fn_decl(fd),
            LocusMember::Const(c) => {
                self.ty(&c.ty);
                self.expr(&c.value);
            }
            LocusMember::Type(t) => self.type_decl(t),
            LocusMember::Capacity(cb) => {
                for slot in &cb.slots {
                    self.ty(&slot.elem_ty);
                }
            }
            LocusMember::Bindings(bb) => {
                for entry in &bb.entries {
                    if let TransportSpec::Adapter { inits, .. } = &entry.transport {
                        self.struct_inits(inits);
                    }
                    if let Some(codec) = &entry.codec {
                        self.struct_inits(&codec.inits);
                    }
                }
                if let Some(api) = &bb.api {
                    match &api.transport {
                        ApiTransport::Unix { path, .. } => self.expr(path),
                    }
                    if let Some(roles) = &api.roles {
                        self.expr(&roles.expr);
                    }
                    if let Some(http) = &api.http {
                        self.expr(&http.host);
                        self.expr(&http.port);
                        self.opt_expr(&http.principals);
                    }
                }
            }
            LocusMember::BirthCheck(bc) => {
                self.expr(&bc.cond);
                self.opt_expr(&bc.payload);
            }
            LocusMember::Placement(_) | LocusMember::Topology(_) | LocusMember::Claims(_) => {}
        }
    }

    fn params_block(&mut self, pb: &ParamsBlock) {
        for p in &pb.params {
            self.opt_ty(&p.ty);
            if let ParamInit::Value(e) = &p.init {
                self.expr(e);
            }
        }
    }

    fn fn_decl(&mut self, fd: &FnDecl) {
        self.generics(&fd.generics);
        self.params(&fd.params);
        self.opt_ty(&fd.ret);
        self.opt_ty(&fd.fallible);
        self.block(&fd.body);
    }

    fn type_decl(&mut self, t: &TypeDecl) {
        self.generics(&t.generics);
        match &t.body {
            TypeDeclBody::Alias(a) => self.ty(a),
            TypeDeclBody::Struct(fields) => {
                for field in fields {
                    self.ty(&field.ty);
                    self.opt_expr(&field.default);
                }
            }
            TypeDeclBody::Enum(variants) => {
                for v in variants {
                    for t in &v.fields {
                        self.ty(t);
                    }
                }
            }
        }
    }

    fn generics(&mut self, gs: &[GenericParam]) {
        for g in gs {
            self.opt_ty(&g.bound);
        }
    }

    fn params(&mut self, ps: &[Param]) {
        for p in ps {
            self.ty(&p.ty);
            self.opt_expr(&p.default);
        }
    }

    fn opt_ty(&mut self, t: &Option<TypeExpr>) {
        if let Some(t) = t {
            self.ty(t);
        }
    }

    fn ty(&mut self, t: &TypeExpr) {
        match t {
            TypeExpr::Named { generic_args, .. } => {
                for a in generic_args {
                    self.ty(a);
                }
            }
            TypeExpr::Projection { inner, .. } => self.ty(inner),
            TypeExpr::Array { elem, size, .. } => {
                self.ty(elem);
                self.opt_expr(size);
            }
            TypeExpr::Bounded { elem, .. } => self.ty(elem),
            TypeExpr::Tuple(elems, _) => {
                for e in elems {
                    self.ty(e);
                }
            }
            TypeExpr::Function { params, ret, .. } => {
                for p in params {
                    self.ty(p);
                }
                if let Some(r) = ret {
                    self.ty(r);
                }
            }
            TypeExpr::Primitive(..) | TypeExpr::Perspective { .. } => {}
        }
    }

    fn block(&mut self, b: &Block) {
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(tail) = &b.tail {
            self.expr(tail);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { ty, value, .. } | Stmt::LetTuple { ty, value, .. } => {
                self.opt_ty(ty);
                self.expr(value);
            }
            Stmt::Assign { target, value, .. } => {
                for seg in &target.tail {
                    if let LValueSeg::Index(e) = seg {
                        self.expr(e);
                    }
                }
                self.expr(value);
            }
            Stmt::If(i) => self.if_stmt(i),
            Stmt::Match(ms) => self.match_stmt(ms, true),
            Stmt::For { iter, body, .. } => {
                self.expr(iter);
                self.block(body);
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond);
                self.block(body);
            }
            Stmt::Return(e, _) => self.opt_expr(e),
            Stmt::Fail { value, .. } => self.expr(value),
            Stmt::Block(b) => self.block(b),
            Stmt::Recovery { args, modifier, .. } => {
                for a in args {
                    self.expr(a);
                }
                if let Some(RecoveryModifier::For(e) | RecoveryModifier::Until(e)) = modifier {
                    self.expr(e);
                }
            }
            Stmt::Violate { payload, .. } => self.opt_expr(payload),
            Stmt::Send { subject, value, or_disposition, .. } => {
                self.expr(subject);
                self.expr(value);
                if let Some(d) = or_disposition {
                    self.disposition(d);
                }
            }
            Stmt::ShmWrite { max, body, .. } => {
                self.expr(max);
                self.block(body);
            }
            Stmt::Expr(e) => self.statement_expr(e),
            Stmt::Break(_)
            | Stmt::Continue(_)
            | Stmt::Yield(_)
            | Stmt::Terminate(_)
            | Stmt::Reperspective { .. } => {}
        }
    }

    /// An expression whose value is dropped, lowered by `lower_stmt_at`:
    /// a call here is at statement position.
    fn statement_expr(&mut self, e: &Expr) {
        match e {
            Expr::Call { callee, args, .. } => self.call(callee, args, Position::Statement),
            other => self.expr(other),
        }
    }

    fn if_stmt(&mut self, i: &IfStmt) {
        self.expr(&i.cond);
        self.block(&i.then_block);
        if let Some(eb) = &i.else_block {
            match &**eb {
                ElseBranch::Else(b) => self.block(b),
                ElseBranch::ElseIf(i) => self.if_stmt(i),
            }
        }
    }

    fn match_stmt(&mut self, ms: &MatchStmt, statement: bool) {
        self.expr(&ms.scrutinee);
        for arm in &ms.arms {
            self.opt_expr(&arm.guard);
            match &arm.body {
                MatchArmBody::Expr(e) if statement => self.statement_expr(e),
                MatchArmBody::Expr(e) => self.expr(e),
                MatchArmBody::Block(b) => self.block(b),
            }
        }
    }

    fn disposition(&mut self, d: &OrDisposition) {
        match d {
            OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => self.expr(e),
            OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) => {}
        }
    }

    fn struct_inits(&mut self, inits: &[StructInit]) {
        for init in inits {
            self.expr(&init.value);
        }
    }

    fn opt_expr(&mut self, e: &Option<Expr>) {
        if let Some(e) = e {
            self.expr(e);
        }
    }

    fn call(&mut self, callee: &Expr, args: &[Expr], position: Position) {
        self.calls += 1;
        if let Expr::Path(qn) = callee {
            if qn.segments.first().is_some_and(|s| s.name == "std") {
                let path = qn.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::");
                self.found.push((path, position, self.owner.clone()));
            }
        }
        self.expr(callee);
        for a in args {
            self.expr(a);
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Call { callee, args, .. } => self.call(callee, args, Position::Expression),
            Expr::Or { inner, disposition, .. } => {
                match &**inner {
                    Expr::Call { callee, args, .. } => self.call(callee, args, Position::Fallible),
                    other => self.expr(other),
                }
                self.disposition(disposition);
            }
            Expr::Struct { inits, .. } => self.struct_inits(inits),
            Expr::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Unary { operand, .. } => self.expr(operand),
            Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. } => self.expr(receiver),
            Expr::Index { receiver, index, .. } => {
                self.expr(receiver);
                self.expr(index);
            }
            Expr::Tuple(parts, _) | Expr::Array(parts, _) => {
                for p in parts {
                    self.expr(p);
                }
            }
            Expr::Block(b) => self.block(b),
            Expr::If(i) => self.if_stmt(i),
            Expr::Match(ms) => self.match_stmt(ms, false),
            Expr::Sum(inner, _) | Expr::Prod(inner, _) => self.expr(inner),
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr(left);
                self.expr(right);
                self.expr(tolerance);
            }
            Expr::Range { lo, hi, .. } => {
                self.expr(lo);
                self.expr(hi);
            }
            Expr::ArrayRepeat { val, .. } => self.expr(val),
            Expr::Ident(_) | Expr::Literal(..) | Expr::Path(_) | Expr::KwSelf(_) => {}
        }
    }
}

/// The site walk's count of calls in `p`.
fn site_calls(p: &Program) -> usize {
    let mut n = 0;
    hale_syntax::sites::for_each_site(p, &mut |kind, _, _| {
        if kind == hale_syntax::sites::SiteKind::Call {
            n += 1;
        }
    });
    n
}

// ---------------------------------------------------------------------
// The shadow set.
// ---------------------------------------------------------------------

/// Where a program of the shadow set comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Corpus,
    Lifecycle,
    TestsHale,
    Dna,
    Harvested,
    Fixtures,
}

impl Source {
    pub fn word(self) -> &'static str {
        match self {
            Source::Corpus => "corpus",
            Source::Lifecycle => "lifecycle",
            Source::TestsHale => "tests/hale",
            Source::Dna => "DNA",
            Source::Harvested => "harvested",
            Source::Fixtures => "stdlib_calls",
        }
    }
}

/// One program of the set: its origin and the parsed files a build of
/// it reads.
pub struct ShadowProgram {
    pub source: Source,
    pub origin: String,
    pub files: Vec<(String, Program)>,
}

fn repo() -> PathBuf {
    hale_corpus::repo_root()
}

fn rel(p: &Path) -> String {
    p.strip_prefix(repo()).unwrap_or(p).display().to_string()
}

fn hl_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut v: Vec<PathBuf> =
        rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "hl")).collect();
    v.sort();
    v
}

/// The files a build of `target` (a file, or a directory seed) reads:
/// the seed's files, then every `import` followed, transitively.
fn program_files(target: &Path) -> Vec<(String, Program)> {
    let mut queue: Vec<PathBuf> = if target.is_dir() { hl_in(target) } else { vec![target.to_path_buf()] };
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    while let Some(f) = queue.pop() {
        let Ok(canon) = f.canonicalize() else { continue };
        if !seen.insert(canon.clone()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&canon) else { continue };
        let Ok(program) = hale_syntax::parse_source(&text) else {
            panic!("{} is in the shadow set and does not parse", rel(&canon));
        };
        let dir = canon.parent().unwrap().to_path_buf();
        for imp in &program.imports {
            let at = dir.join(&imp.path);
            if at.is_dir() {
                queue.extend(hl_in(&at));
            } else if at.exists() {
                queue.push(at);
            } else if at.with_extension("hl").exists() {
                queue.push(at.with_extension("hl"));
            }
        }
        out.push((rel(&canon), program));
    }
    out
}

fn dna_mains(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut es: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
    es.sort();
    for p in es {
        if p.is_dir() {
            dna_mains(&p, out);
        } else if p.file_name().is_some_and(|n| n == "main.hl") {
            out.push(p.parent().unwrap().to_path_buf());
        }
    }
}

/// The harvested programs the shadow builds: every program
/// `hale_corpus::embedded` scrapes out of a Rust test that parses, is
/// one file (no sibling-seed `import`), names no `@ffi` host import
/// (whose host side is the test's), and checks clean — a program the
/// checker refuses never reaches lowering. Each with the entry point
/// `corpus_check_build_agreement` appends when it has none.
pub fn harvested() -> Vec<(String, String)> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for p in hale_corpus::embedded() {
        if !seen.insert(p.source.clone()) || p.source.contains("import \"") || p.source.contains("@ffi(") {
            continue;
        }
        let Ok(program) = hale_syntax::parse_source(&p.source) else { continue };
        if hale_types::check_program(&program).iter().any(|d| d.is_error()) {
            continue;
        }
        let has_entry = program.items.iter().any(|i| match i {
            TopDecl::Fn(f) => f.name.name == "main",
            TopDecl::Locus(l) => l.is_main,
            _ => false,
        });
        let source = if has_entry { p.source } else { format!("{}\nfn main() {{ }}\n", p.source) };
        out.push((p.origin, source));
    }
    out
}

pub fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stdlib_calls")
}

pub fn shadow_set() -> Vec<ShadowProgram> {
    let root = repo();
    let mut out = Vec::new();
    let codegen = root.join("crates/hale-codegen/tests/fixtures");
    let mut push = |source: Source, target: &Path| {
        out.push(ShadowProgram { source, origin: rel(target), files: program_files(target) });
    };
    let examples = codegen.join("examples");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&examples).unwrap().flatten().map(|e| e.path()).collect();
    entries.sort();
    for e in entries.iter().filter(|p| p.is_dir() || p.extension().is_some_and(|x| x == "hl")) {
        push(Source::Corpus, e);
    }
    for f in hl_in(&codegen.join("lifecycle")) {
        push(Source::Lifecycle, &f);
    }
    for f in hl_in(&root.join("tests/hale")) {
        push(Source::TestsHale, &f);
    }
    let mut mains = Vec::new();
    dna_mains(&root.join("dna"), &mut mains);
    for m in &mains {
        push(Source::Dna, m);
    }
    for f in hl_in(&fixtures_dir()) {
        push(Source::Fixtures, &f);
    }
    for (origin, source) in harvested() {
        let program = hale_syntax::parse_source(&source).expect("harvested programs parse");
        out.push(ShadowProgram { source: Source::Harvested, origin: origin.clone(), files: vec![(origin, program)] });
    }
    out
}

/// Each pair the shadow set calls, with the programs (by source) that
/// call it. A statement call of a path with no statement branch covers
/// the expression arm that lowers it.
pub fn coverage(set: &[ShadowProgram]) -> BTreeMap<(String, Position), BTreeSet<(Source, String)>> {
    let scraped = scrape();
    let statement = statement_scrape(&scraped);
    let mut out: BTreeMap<(String, Position), BTreeSet<(Source, String)>> = BTreeMap::new();
    let mut walked = BTreeSet::new();
    for prog in set {
        for (file, program) in &prog.files {
            let w = CallWalk::program(program);
            if walked.insert(file.clone()) {
                assert_eq!(
                    w.calls,
                    site_calls(program),
                    "{file}: the coverage walk met a different number of calls than \
                     `hale_syntax::sites` — it skipped (or invented) an expression"
                );
            }
            for (path, position, _) in w.found {
                for pair in covers(path, position, statement) {
                    out.entry(pair).or_default().insert((prog.source, prog.origin.clone()));
                }
            }
        }
    }
    out
}

/// The std calls of the Hale-source stdlib (`hale_stdlib::AP_SOURCE`),
/// by the pairs they cover, with the declarations that make them.
pub fn stdlib_calls() -> BTreeMap<(String, Position), BTreeSet<String>> {
    let scraped = scrape();
    let statement = statement_scrape(&scraped);
    let program = hale_syntax::parse_source(hale_stdlib::AP_SOURCE).expect("the stdlib parses");
    let w = CallWalk::program(&program);
    assert_eq!(w.calls, site_calls(&program), "the stdlib: the coverage walk missed a call");
    let mut out: BTreeMap<(String, Position), BTreeSet<String>> = BTreeMap::new();
    for (path, position, owner) in w.found {
        for pair in covers(path, position, statement) {
            out.entry(pair).or_default().insert(owner.clone());
        }
    }
    out
}

// ---------------------------------------------------------------------
// The tests.
// ---------------------------------------------------------------------

/// The scrape reads what it thinks it reads: each dispatcher has arms,
/// the families expanded to names, and the walk classifies a known
/// program the way lowering would.
#[test]
fn the_scrape_and_the_walk_are_not_vacuous() {
    let scraped = scrape();
    for s in &scraped {
        // The statement position has arms only where a statement
        // answers differently (21 after S1) and drops the expression
        // position's value everywhere else.
        let floor = if s.position == Position::Statement { 15 } else { 50 };
        assert!(s.arms.len() > floor, "{:?}: only {} arms scraped", s.position, s.arms.len());
        assert!(s.shadowed.is_empty(), "{:?}: arms no call reaches: {:?}", s.position, s.shadowed);
    }
    let all = pairs(&scraped);
    assert!(all.contains_key(&("std::io::sockopt::SOL_SOCKET".to_string(), Position::Expression)));
    assert!(all.contains_key(&("std::bytes::read_u8".to_string(), Position::Fallible)));
    assert!(all.contains_key(&("std::io::mirror::__new".to_string(), Position::Expression)));
    assert_eq!(
        all.get(&("std::str::parse_int".to_string(), Position::Expression)).map(|k| k.0),
        Some(ArmKind::Refuses)
    );
    assert_eq!(
        all.get(&("std::tar::pack".to_string(), Position::Expression)).map(|k| k.0),
        Some(ArmKind::Refuses)
    );
    assert_eq!(
        all.get(&("std::math::sqrt".to_string(), Position::Fallible)).map(|k| k.0),
        Some(ArmKind::Refuses)
    );
    assert!(!all.contains_key(&("std::process::run".to_string(), Position::Statement)));
    assert!(!all.contains_key(&("std::str::parse_int".to_string(), Position::Statement)));
    assert_eq!(all.get(&("std::json::valid".to_string(), Position::Expression)).map(|k| k.0), Some(ArmKind::Lowers));
    assert_eq!(all.get(&("std::test::assert".to_string(), Position::Statement)).map(|k| k.0), Some(ArmKind::Lowers));
    assert!(!all.contains_key(&("std::test::assert".to_string(), Position::Expression)));
    let statement = &statement_scrape(&scraped).paths;
    assert_eq!(lowered_at("std::time::sleep", Position::Statement, statement), Position::Statement);
    assert_eq!(lowered_at("std::ring::__spsc_emit", Position::Statement, statement), Position::Statement);
    assert_eq!(lowered_at("std::str::contains", Position::Statement, statement), Position::Expression);
    assert_eq!(lowered_at("std::str::contains", Position::Expression, statement), Position::Expression);
    let p = hale_syntax::parse_source(
        "fn f() -> Int {\n    std::time::sleep(1);\n    let n = std::str::len(\"ab\");\n    \
         let k = std::str::parse_int(\"1\") or 0;\n    match n {\n        2 -> std::process::exit(0),\n        \
         _ -> {}\n    }\n    std::str::len(\"abc\")\n}\n",
    )
    .unwrap();
    let w = CallWalk::program(&p);
    let found: Vec<(&str, Position)> = w.found.iter().map(|(p, pos, _)| (p.as_str(), *pos)).collect();
    assert_eq!(
        found,
        vec![
            ("std::time::sleep", Position::Statement),
            ("std::str::len", Position::Expression),
            ("std::str::parse_int", Position::Fallible),
            ("std::process::exit", Position::Statement),
            ("std::str::len", Position::Expression),
        ]
    );
    assert_eq!(w.calls, site_calls(&p));
}

/// Write the harvested programs of the shadow set out as files, one per
/// program, under `<target>/s0-shadow/harvest/`, for the IR shadow
/// runner (which builds files, not strings) to build like the rest of
/// the set. Not a check: run it explicitly before a shadow pass.
#[test]
#[ignore = "writes the harvested programs for the IR shadow runner"]
fn write_harvested_programs() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).parent().unwrap().join("s0-shadow/harvest");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let programs = harvested();
    for (origin, source) in &programs {
        let name: String =
            origin.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
        std::fs::write(dir.join(format!("{name}.hl")), source).unwrap();
    }
    eprintln!("wrote {} harvested programs to {}", programs.len(), dir.display());
}

// ---------------------------------------------------------------------
// The allowances: pairs no checked program can reach, each kind with
// the reason from the code, each list held to what it claims.
// ---------------------------------------------------------------------

/// Allowance 1, refused by lowering, read from the rows (S5; until then
/// three hand-kept lists): a bare call of a function whose row says it can
/// fail (`bare_call_of_a_fallible_row`, the value position's arm, which a
/// statement reaches too), and an `or` over one whose row says it cannot
/// (`or_over_an_infallible_row`). No program builds with one, and the
/// check refuses all but an `or` over a row with no signature yet. The
/// rows' one exception is the arm a later ruling of S5 removes: the
/// infallible row an `or` still lowers.
pub fn refused_by_the_rows() -> BTreeSet<(String, Position)> {
    use hale_types::stdlib_surface::{rows, Lower};
    rows()
        .filter(|(_, f)| matches!(f.lower, Lower::Intrinsic(_)))
        .map(|(s, f)| {
            let path = format!("std::{}::{}", s.ns.join("::"), f.name);
            let fallible = f.sig.is_some_and(|s| s.fallible.is_some());
            (path, if fallible { Position::Expression } else { Position::Fallible })
        })
        .filter(|(path, position)| {
            *position == Position::Expression || !OR_LOWERS_AN_INFALLIBLE_ROW.contains(&path.as_str())
        })
        .collect()
}

/// The infallible row an `or` still lowers: `ecdsa_p256_sign`, whose bare
/// call returns empty `Bytes` and whose `or` call can fail. Ruling 4 makes
/// its row fallible.
pub const OR_LOWERS_AN_INFALLIBLE_ROW: &[&str] = &["std::crypto::ecdsa_p256_sign"];

/// Allowance 2, internal: paths the checker refuses from a user's
/// program ("unknown stdlib function": they are in no registry row, and
/// `std::bus` and `std::test` are tabled namespaces), so only the
/// stdlib's own `.hl` seeds call them. Each is reached through the
/// stdlib declarations named beside it, which call it at that position
/// and which every build lowers (the IR is dumped before dead code is
/// dropped): the test builds a program with an empty `main` and finds
/// each one defined. So every program of the shadow set carries these
/// arms' IR.
const INTERNAL: &[(&str, Position, &[&str])] = &[
    ("std::bus::__binding_fail", Position::Statement, &["__StdBusUnixConnectTransport", "__StdBusUnixListenTransport"]),
    ("std::bus::__transport_realize", Position::Expression, &["__StdBusUnixConnectTransport", "__StdBusUnixListenTransport"]),
    ("std::bus::__transport_reclaim", Position::Statement, &["__StdBusUnixConnectTransport", "__StdBusUnixListenTransport"]),
    ("std::bus::__transport_spawn_server", Position::Expression, &["__StdBusUnixListenTransport"]),
    ("std::test::__failed", Position::Expression, &["__test_assert", "__test_assert_eq_int", "__test_assert_eq_str"]),
    ("std::test::__note_fail", Position::Statement, &["__test_assert", "__test_assert_eq_int", "__test_assert_eq_str"]),
    ("std::test::__note_pass", Position::Statement, &["__test_assert", "__test_assert_eq_int", "__test_assert_eq_str"]),
    ("std::test::__passes", Position::Expression, &["__test_fail_trailer"]),
];

/// Every allowed pair, with the kind it is allowed as.
fn allowances() -> BTreeMap<(String, Position), &'static str> {
    let mut out = BTreeMap::new();
    for pair in refused_by_the_rows() {
        out.insert(pair, "refused by lowering");
    }
    for (p, position, _) in INTERNAL {
        out.insert((p.to_string(), *position), "internal");
    }
    out
}

/// The allowances say what the code says: the refused pairs are exactly
/// the refusal arms' pairs, which are the rows' fallibility; each internal
/// path is refused to a user's program and called, at its position, by
/// the stdlib declarations named, all lowered in every build. No pair
/// reaches "not implemented", and since S5 no fallible row keeps a bare
/// arm that lowers (the nine dead ones are gone).
#[test]
fn the_allowances_are_what_the_code_says() {
    let all = pairs(&scrape());
    let allowed = allowances();
    let refused_scraped: BTreeSet<(String, Position)> =
        all.iter().filter(|(_, k)| k.0 == ArmKind::Refuses).map(|(p, _)| p.clone()).collect();
    let refused_rows: BTreeSet<(String, Position)> =
        allowed.iter().filter(|(_, why)| **why == "refused by lowering").map(|(p, _)| p.clone()).collect();
    assert_eq!(refused_rows, refused_scraped, "the refusal arms drifted from the rows' fallibility");

    let seeds = stdlib_calls();
    let ir = harness::build_ir_text(
        &hale_syntax::parse_source("fn main() { }\n").unwrap(),
        &harness::unique_bin("stdlib_dispatch_coverage_empty_main"),
    )
    .expect("an empty main builds");
    for (path, position, callers) in INTERNAL {
        let segs: Vec<&str> = path.split("::").collect();
        assert!(all.contains_key(&(path.to_string(), *position)), "{path}: not a dispatched pair");
        assert!(
            hale_types::stdlib_surface::unknown_fn_error(&segs).is_some(),
            "{path}: the checker accepts it from a user's program, so a fixture can call it"
        );
        let found: BTreeSet<&str> = seeds
            .get(&(path.to_string(), *position))
            .map(|s| s.iter().map(String::as_str).collect())
            .unwrap_or_default();
        assert_eq!(found, callers.iter().copied().collect(), "{path}: the stdlib's callers");
        for caller in *callers {
            assert!(
                ir.lines().any(|l| l.starts_with("define ")
                    && (l.contains(&format!("@{caller}(")) || l.contains(&format!("@{caller}.")))),
                "{caller} (calls {path}) is not lowered in a build of an empty main"
            );
        }
    }
}

/// The shadow set reaches every (path, position) pair a dispatcher
/// matches but the allowed ones: the gate line S's IR comparison stands
/// on. A failure names each pair no program of the set calls at that
/// position, with its arm's line, and each allowance a program now
/// reaches (the list is kept exact). A program counts here without
/// being built: one that does not build gives the shadow a build log
/// and no IR (the shadow pass shows it as an empty IR), so a pair must
/// not rest on such a program alone — when S0 measured, one did
/// (`std::regex::matches`, a DNA example), and a fixture now calls it.
#[test]
fn every_dispatch_pair_is_reached_by_the_shadow_set() {
    let all = pairs(&scrape());
    let cov = coverage(&shadow_set());
    let allowed = allowances();
    let uncovered: Vec<String> = all
        .iter()
        .filter(|(pair, _)| !cov.contains_key(*pair) && !allowed.contains_key(*pair))
        .map(|((path, position), (kind, line))| {
            format!("{path} at {} position ({kind:?}, arm at line {line})", position.word())
        })
        .collect();
    assert!(
        uncovered.is_empty(),
        "{} of {} stdlib dispatch pairs are called by no program of the shadow set:\n  {}",
        uncovered.len(),
        all.len(),
        uncovered.join("\n  ")
    );
    let stale: Vec<String> = allowed
        .iter()
        .filter(|(pair, _)| !all.contains_key(*pair) || cov.contains_key(*pair))
        .map(|((path, position), why)| format!("{path} at {} position ({why})", position.word()))
        .collect();
    assert!(stale.is_empty(), "allowances that are no dispatched pair, or that a program now reaches:\n  {}", stale.join("\n  "));
}

/// The measurements line S's plan quotes, re-measured: printed, not
/// asserted. `-- --ignored --nocapture` to read them.
#[test]
#[ignore = "prints the dispatcher and coverage measurements"]
fn report_the_dispatchers_and_the_coverage() {
    let scraped = scrape();
    let mut by_path: BTreeMap<String, BTreeSet<Position>> = BTreeMap::new();
    for s in &scraped {
        eprintln!(
            "{:<11} {:>3} arms; {:>3} paths ({} refused); {} shadowed {:?}",
            s.position.word(),
            s.arms.len(),
            s.paths.len(),
            s.paths.values().filter(|k| k.0 == ArmKind::Refuses).count(),
            s.shadowed.len(),
            s.shadowed,
        );
        for p in s.paths.keys() {
            by_path.entry(p.clone()).or_default().insert(s.position);
        }
    }
    let multi = by_path.values().filter(|ps| ps.len() >= 2).count();
    let all3 = by_path.values().filter(|ps| ps.len() == 3).count();
    eprintln!("union {} paths; {} at two or more positions ({} at all three)", by_path.len(), multi, all3);
    let all = pairs(&scraped);
    let set = shadow_set();
    let mut per_source: BTreeMap<Source, usize> = BTreeMap::new();
    for p in &set {
        *per_source.entry(p.source).or_default() += 1;
    }
    eprintln!("shadow set: {} programs {:?}", set.len(), per_source);
    let cov = coverage(&set);
    let covered: BTreeSet<&(String, Position)> = all.keys().filter(|k| cov.contains_key(*k)).collect();
    eprintln!("pairs: {} ({} lowering, {} refused); covered {}", all.len(),
        all.values().filter(|k| k.0 == ArmKind::Lowers).count(),
        all.values().filter(|k| k.0 == ArmKind::Refuses).count(),
        covered.len());
    for src in [Source::Corpus, Source::Lifecycle, Source::TestsHale, Source::Dna, Source::Harvested, Source::Fixtures] {
        let by = all.keys().filter(|k| cov.get(*k).is_some_and(|s| s.iter().any(|(x, _)| *x == src))).count();
        let only = all
            .keys()
            .filter(|k| cov.get(*k).is_some_and(|s| s.iter().all(|(x, _)| *x == src)))
            .count();
        eprintln!("  covered by {:<12} {:>3} (only by it: {})", src.word(), by, only);
    }
    for ((path, position), (kind, _)) in &all {
        eprintln!("  pair {path} {} {kind:?}", position.word());
    }
    let seeds = stdlib_calls();
    for ((path, position), (kind, line)) in &all {
        if cov.contains_key(&(path.clone(), *position)) {
            continue;
        }
        let seed = seeds.get(&(path.clone(), *position));
        eprintln!(
            "  uncovered {:<55} {:<10} {:?} line {} {} {}",
            path,
            position.word(),
            kind,
            line,
            signature_text(path),
            seed.map(|s| format!("stdlib callers: {s:?}")).unwrap_or_default()
        );
    }
}

/// A path's signature row, written out (`-` when it has none).
fn signature_text(path: &str) -> String {
    use hale_types::stdlib_surface::SigTy;
    let t = |s: &SigTy| match s {
        SigTy::Int => "Int".to_string(),
        SigTy::Uint => "Uint".to_string(),
        SigTy::Float => "Float".to_string(),
        SigTy::Bool => "Bool".to_string(),
        SigTy::Str => "String".to_string(),
        SigTy::Bytes => "Bytes".to_string(),
        SigTy::BytesMut => "BytesMut".to_string(),
        SigTy::Decimal => "Decimal".to_string(),
        SigTy::Duration => "Duration".to_string(),
        SigTy::Time => "Time".to_string(),
        SigTy::Unit => "()".to_string(),
        SigTy::Any => "_".to_string(),
        SigTy::Named(n) => n.to_string(),
    };
    let segs: Vec<&str> = path.split("::").collect();
    match hale_types::stdlib_surface::signature_for(&segs) {
        None => "-".to_string(),
        Some(s) => format!(
            "({}) -> {}{}",
            s.params.iter().map(t).collect::<Vec<_>>().join(", "),
            t(&s.ret),
            s.fallible.map(|f| format!(" fallible({f})")).unwrap_or_default()
        ),
    }
}
