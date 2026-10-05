//! F.40 phase 4, line S, step S0: every stdlib dispatch arm is reached
//! by the shadow set at every position it is dispatched from.
//!
//! **Scaffolding.** Line S moves the stdlib call paths out of the three
//! hand-written dispatchers (`lower_stdlib_path_call`, statement
//! position; `lower_stdlib_path_call_expr`, expression position;
//! `try_lower_fallible_stdlib_path_call`, the `or` position) into one
//! table-driven `match`, with IR identity over the shadow set as the
//! only proof that nothing changed. That proof is only as good as the
//! set's reach, so this file holds it to every arm before an arm moves.
//! It scrapes the `["std", ..]` literals out of the dispatchers' source,
//! so it is deleted when those literals are gone (S3/S4).
//!
//! A *pair* is a (path, position): a stdlib call path a dispatcher
//! matches, and which dispatcher matched it. The *shadow set* is what
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

/// Which dispatcher a `std::` path call reaches. The rule, mirrored by
/// `CallWalk` below, lives in `crates/hale-codegen/src/codegen.rs`:
///
/// * `lower_stmt_at`'s `Stmt::Expr(Expr::Call { callee: Expr::Path .. })`
///   arm calls `lower_path_call`, which sends a `std` path to
///   `lower_stdlib_path_call`: a call that IS a statement. A
///   statement-position `match` whose arm body is a call expression
///   routes that body through `lower_stmt` too (`lower_match_core`,
///   `capture: None`), so it is a statement as well. That dispatcher's
///   last arm hands every path it has no arm for to
///   `lower_stdlib_path_call_expr` and drops the value (S1), so such a
///   statement is lowered by an EXPRESSION arm: [`lowered_at`] says
///   which, and the call covers that arm's pair and the fall-through
///   arm's ([`FALL_THROUGH`]).
/// * `lower_or_expr` (`channels/mod.rs`, reached from the `Stmt::Expr(
///   Expr::Or ..)` statement and from `lower_expr`'s `Expr::Or`) calls
///   `lower_fallible_call`, whose `Expr::Path` callee goes to
///   `try_lower_fallible_stdlib_path_call`: the call directly under an
///   `or`, wherever the `or` stands.
/// * Every other call — an argument, a `let` value, a block's tail
///   (`lower_block` and `lower_block_as_expr` both lower the tail with
///   `lower_expr`), a value-producing `match` arm — goes through
///   `lower_expr`'s `Expr::Call` arm to `lower_path_call_expr` and
///   `lower_stdlib_path_call_expr`.
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

/// The three dispatchers: the function, the file it is in, the
/// position it serves.
const DISPATCHERS: &[(&str, &str, Position)] = &[
    ("lower_stdlib_path_call", "src/codegen.rs", Position::Statement),
    ("lower_stdlib_path_call_expr", "src/codegen.rs", Position::Expression),
    ("try_lower_fallible_stdlib_path_call", "src/channels/mod.rs", Position::Fallible),
];

/// The pair of the statement dispatcher's last arm, which lowers every
/// path the dispatcher has no arm for through the expression dispatcher
/// (`let _ = self.lower_stdlib_path_call_expr(segs, args, scope)?`). Not
/// a path: it stands for that one arm, covered by any statement call
/// that falls through.
pub const FALL_THROUGH: &str = "std::_";

/// The position whose dispatcher arm lowers a call the walk found at
/// `position`: a statement call of a path the statement dispatcher has
/// no arm for falls through to the expression dispatcher.
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

/// One segment of a slice pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Seg {
    Lit(String),
    Bind(String),
}

/// One arm of a dispatcher's top-level `match segs`.
#[derive(Clone, Debug)]
pub struct Arm {
    /// 1-indexed line of the arm's first pattern in its file.
    pub line: usize,
    patterns: Vec<Vec<Seg>>,
    guard: Option<String>,
    /// The arm's value is an `Err(..)`: one of the refusal lists.
    pub refuses: bool,
}

/// What a family pattern (a pattern whose last segment binds a name)
/// accepts, read from where the name is decided.
#[derive(Default)]
struct Families {
    /// `SOCKOPT_NAMES` in codegen.rs.
    sockopt: Vec<String>,
    /// The string arms of `lower_std_io_mirror`'s `match op`.
    mirror: Vec<String>,
}

fn families() -> Families {
    let src = crate_file("src/codegen.rs");
    let code = mask(&src, false);
    let at = code.find("const SOCKOPT_NAMES: &[&str] = &[").expect("SOCKOPT_NAMES");
    let end = at + code[at..].find("];").expect("SOCKOPT_NAMES ends");
    let sockopt = string_literals(&code[at..end]);
    let mirror_src = crate_file("src/stdlib/mirror.rs");
    let mirror_code = mask(&mirror_src, false);
    // The impl's definition is the second `fn lower_std_io_mirror(`
    // (the first is the trait's declaration, which has no body).
    let impl_at = mirror_code.rfind("fn lower_std_io_mirror(").expect("mirror impl");
    let (o, c) = fn_body(&mirror_code[impl_at..], "lower_std_io_mirror");
    let body = &mirror_code[impl_at + o..impl_at + c];
    let m = body.find("match op {").expect("mirror's match op");
    let indent = line_indent(body, m) + 4;
    let mut mirror = Vec::new();
    for line in body[m..].lines().skip(1) {
        if indent_of(line) == indent && line.trim_start().starts_with('"') {
            mirror.extend(string_literals(line.split("=>").next().unwrap()));
        }
    }
    Families { sockopt, mirror }
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

/// The arms of one dispatcher, in source order.
pub fn arms_of(name: &str, file: &str) -> Vec<Arm> {
    let src = crate_file(file);
    let no_comments = mask(&src, false);
    let code = mask(&src, true);
    let (open, close) = fn_body(&code, name);
    let body = &no_comments[open..close];
    let first_line = src[..open].matches('\n').count() + 1;
    let m = body.find("match segs {").unwrap_or_else(|| panic!("`{name}` matches on `segs`"));
    let arm_indent = line_indent(body, m) + 4;
    let lines: Vec<&str> = body.lines().collect();
    let mut arms = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        if indent_of(l) != arm_indent || !l.trim_start().starts_with("[\"std\"") {
            i += 1;
            continue;
        }
        let start = i;
        let mut head = String::new();
        loop {
            let l = lines[i];
            if let Some(k) = l.find("=>") {
                head.push_str(&l[..k]);
                head.push(' ');
                let rest = l[k + 2..].trim_start().trim_start_matches('{').trim_start();
                let rest = if rest.is_empty() {
                    lines.get(i + 1).map_or("", |n| n.trim_start())
                } else {
                    rest
                };
                let (patterns, guard) = parse_head(&head);
                arms.push(Arm {
                    line: first_line + start,
                    patterns,
                    guard,
                    refuses: rest.starts_with("Err("),
                });
                break;
            }
            head.push_str(l);
            head.push(' ');
            i += 1;
        }
        i += 1;
    }
    arms
}

fn parse_head(head: &str) -> (Vec<Vec<Seg>>, Option<String>) {
    let mut patterns = Vec::new();
    let mut rest = head;
    while let Some(o) = rest.find('[') {
        let c = o + rest[o..].find(']').expect("a pattern closes");
        let segs = rest[o + 1..c]
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| match s.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                Some(lit) => Seg::Lit(lit.to_string()),
                None => Seg::Bind(s.to_string()),
            })
            .collect();
        patterns.push(segs);
        rest = &rest[c + 1..];
    }
    let guard = rest.trim();
    let guard = guard.strip_prefix("if ").map(|g| g.trim().to_string());
    (patterns, guard)
}

/// The paths one pattern accepts, its family expanded through what
/// decides the bound name.
fn expand(pattern: &[Seg], guard: Option<&str>, fam: &Families) -> Vec<String> {
    let lits: Vec<&str> = pattern
        .iter()
        .filter_map(|s| match s {
            Seg::Lit(l) => Some(l.as_str()),
            Seg::Bind(_) => None,
        })
        .collect();
    if lits.len() == pattern.len() {
        return vec![lits.join("::")];
    }
    let ns = lits.join("::");
    let leaves: Vec<String> = match (ns.as_str(), guard) {
        // `["std", "bytes", n] if n.starts_with("read_")` (and
        // `write_`): `lower_std_bytes_read`/`_write` parse the name, so
        // the family is what the registry lists under the prefix.
        (_, Some(g)) if g.contains(".starts_with(\"") => {
            let prefix = g.split('"').nth(1).expect("a prefix literal");
            let surface_ns: Vec<&str> = lits[1..].to_vec();
            hale_types::stdlib_surface::SURFACES
                .iter()
                .filter(|s| s.ns == surface_ns.as_slice())
                .flat_map(|s| s.fns.iter().map(|e| e.name))
                .filter(|n| n.starts_with(prefix))
                .map(str::to_string)
                .collect()
        }
        ("std::io::sockopt", Some(g)) if g.contains("SOCKOPT_NAMES.contains") => fam.sockopt.clone(),
        ("std::io::mirror", None) => fam.mirror.clone(),
        _ => panic!("a family pattern this scrape does not know how to expand: {pattern:?} if {guard:?}"),
    };
    assert!(!leaves.is_empty(), "family `{ns}::*` expanded to nothing");
    leaves.into_iter().map(|l| format!("{ns}::{l}")).collect()
}

/// What an arm does with a path: lower it, or refuse it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArmKind {
    Lowers,
    Refuses,
}

/// One dispatcher, scraped.
pub struct Scraped {
    pub position: Position,
    pub arms: Vec<Arm>,
    /// Every path a pattern names (families expanded), with the kind of
    /// the FIRST arm matching it (the one `match` takes) and its line.
    pub paths: BTreeMap<String, (ArmKind, usize)>,
    /// Patterns whose every path an earlier arm already matched: dead.
    pub shadowed: Vec<(String, usize)>,
    pub patterns: usize,
    pub family_patterns: usize,
    /// The line of the call that hands every other path to the
    /// expression dispatcher, when this dispatcher falls through to it.
    pub falls_through: Option<usize>,
}

/// The line of `fn name`'s call of the expression dispatcher, if it
/// makes one.
fn fall_through_line(name: &str, file: &str) -> Option<usize> {
    let src = crate_file(file);
    let code = mask(&src, true);
    let (open, close) = fn_body(&code, name);
    let at = code[open..close].find("self.lower_stdlib_path_call_expr(")?;
    Some(src[..open + at].matches('\n').count() + 1)
}

pub fn scrape() -> Vec<Scraped> {
    let fam = families();
    DISPATCHERS
        .iter()
        .map(|&(name, file, position)| {
            let arms = arms_of(name, file);
            let falls_through = fall_through_line(name, file);
            let mut paths = BTreeMap::new();
            let mut shadowed = Vec::new();
            let (mut patterns, mut family_patterns) = (0, 0);
            for arm in &arms {
                let kind = if arm.refuses { ArmKind::Refuses } else { ArmKind::Lowers };
                for p in &arm.patterns {
                    patterns += 1;
                    if p.iter().any(|s| matches!(s, Seg::Bind(_))) {
                        family_patterns += 1;
                    }
                    for path in expand(p, arm.guard.as_deref(), &fam) {
                        if paths.contains_key(&path) {
                            shadowed.push((path, arm.line));
                        } else {
                            paths.insert(path, (kind, arm.line));
                        }
                    }
                }
            }
            Scraped { position, arms, paths, shadowed, patterns, family_patterns, falls_through }
        })
        .collect()
}

/// Every (path, position) pair, with what its arm does, and the
/// statement dispatcher's fall-through arm as the pair
/// ([`FALL_THROUGH`], statement).
pub fn pairs(scraped: &[Scraped]) -> BTreeMap<(String, Position), (ArmKind, usize)> {
    let mut out = BTreeMap::new();
    for s in scraped {
        for (path, kind) in &s.paths {
            out.insert((path.clone(), s.position), *kind);
        }
        if let Some(line) = s.falls_through {
            assert_eq!(s.position, Position::Statement, "only the statement dispatcher falls through");
            out.insert((FALL_THROUGH.to_string(), s.position), (ArmKind::Lowers, line));
        }
    }
    out
}

/// The statement dispatcher's own paths (its arms', not the ones it
/// hands on).
fn statement_paths(scraped: &[Scraped]) -> &BTreeMap<String, (ArmKind, usize)> {
    &scraped.iter().find(|s| s.position == Position::Statement).expect("a statement dispatcher").paths
}

/// The pairs one walked call covers: the arm that lowers it, and the
/// fall-through arm when it is a statement handed on.
fn covers(
    path: String,
    position: Position,
    statement_paths: &BTreeMap<String, (ArmKind, usize)>,
) -> Vec<(String, Position)> {
    let at = lowered_at(&path, position, statement_paths);
    if at == position {
        vec![(path, position)]
    } else {
        vec![(FALL_THROUGH.to_string(), Position::Statement), (path, at)]
    }
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
/// call it. A statement call the statement dispatcher hands on covers
/// the expression arm that lowers it, and the fall-through arm.
pub fn coverage(set: &[ShadowProgram]) -> BTreeMap<(String, Position), BTreeSet<(Source, String)>> {
    let scraped = scrape();
    let statement = statement_paths(&scraped);
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
    let statement = statement_paths(&scraped);
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
        // The statement dispatcher keeps only the arms a statement
        // answers differently (21 after S1) and hands the rest on.
        let floor = if s.position == Position::Statement { 15 } else { 50 };
        assert!(s.arms.len() > floor, "{:?}: only {} arms scraped", s.position, s.arms.len());
    }
    let all = pairs(&scraped);
    assert!(all.contains_key(&("std::io::sockopt::SOL_SOCKET".to_string(), Position::Expression)));
    assert!(all.contains_key(&("std::bytes::read_u8".to_string(), Position::Fallible)));
    assert!(all.contains_key(&("std::io::mirror::__new".to_string(), Position::Expression)));
    assert_eq!(
        all.get(&("std::str::parse_int".to_string(), Position::Expression)).map(|k| k.0),
        Some(ArmKind::Refuses)
    );
    assert!(all.contains_key(&(FALL_THROUGH.to_string(), Position::Statement)));
    let statement = statement_paths(&scraped);
    assert_eq!(lowered_at("std::time::sleep", Position::Statement, statement), Position::Statement);
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

/// Allowance 1, refused by lowering. The paths of the statement and
/// expression dispatchers' fallibility refusal list (`lower_stdlib_path_
/// call`'s and `lower_stdlib_path_call_expr`'s `Err(".. returns a
/// fallible value — address the error with `or raise` ..")` arm): a bare
/// call of a path lowering treats as fallible is refused at both
/// positions, so no program builds with one. Most have no signature
/// row, so the checker does not refuse them first.
const REFUSED_BARE: &[&str] = &[
    "std::io::file::__open",
    "std::io::file::__seek",
    "std::io::file::__write_bytes",
    "std::io::fs::mktemp",
    "std::io::fs::rename",
    "std::io::fs::unlink",
    "std::io::tls::set_recv_timeout",
    "std::io::tls::set_send_timeout",
    "std::io::udp::__bind",
    "std::io::udp::__recv",
    "std::io::udp::__send",
    "std::io::udp::bind",
    "std::io::udp::get_option_int",
    "std::io::udp::join_group",
    "std::io::udp::leave_group",
    "std::io::udp::recv",
    "std::io::udp::recv_with_source",
    "std::io::udp::send",
    "std::io::udp::set_multicast_iface",
    "std::io::udp::set_multicast_loop",
    "std::io::udp::set_multicast_ttl",
    "std::io::udp::set_option_bool",
    "std::io::udp::set_option_int",
    "std::io::udp::set_recv_timeout",
    "std::io::udp::set_send_timeout",
    "std::os::getrandom",
    "std::process::__kill_escalate",
    "std::process::__pipe_read",
    "std::process::__pipe_write",
    "std::process::__signal_pid",
    "std::process::__spawn",
    "std::process::__try_wait_pid",
    "std::process::__wait_pid",
    "std::process::run",
];

/// Allowance 1, refused by lowering, expression position only: the
/// expression dispatcher's list also refuses the two parsers, which the
/// statement dispatcher has no arm for, so a bare statement call of one
/// falls through and is refused by this same arm.
const REFUSED_BARE_EXPRESSION_ONLY: &[&str] = &["std::str::parse_float", "std::str::parse_int"];

/// Allowance 1, refused by lowering. The fallible dispatcher's list
/// (`try_lower_fallible_stdlib_path_call`'s `Err(".. is not a fallible
/// call — remove the `or` clause ..")` arm): an `or` over a path that
/// returns its value directly is refused, so no program builds with one.
const REFUSED_UNDER_OR: &[&str] = &[
    "std::bytes::__is_alloc_fail",
    "std::bytes::builder::__append",
    "std::bytes::builder::__append_slice",
    "std::bytes::builder::__append_str",
    "std::bytes::builder::__clear",
    "std::bytes::builder::__finish",
    "std::bytes::builder::__free",
    "std::bytes::builder::__len",
    "std::bytes::builder::__new",
    "std::bytes::builder::__shift_front",
    "std::bytes::builder::__snapshot",
    "std::bytes::builder::__text_view",
    "std::bytes::builder::__view",
    "std::bytes::clone",
    "std::bytes::from_string",
    "std::bytes::slice",
    "std::env::arg",
    "std::env::arg_or",
    "std::env::args_count",
    "std::env::var",
    "std::env::var_exists",
    "std::io::fs::file_exists",
    "std::io::stdin::read_line",
    "std::io::stdin::read_line_status",
    "std::io::tcp::close_fd",
    "std::math::acos",
    "std::math::asin",
    "std::math::atan",
    "std::math::atan2",
    "std::math::ceil",
    "std::math::cos",
    "std::math::exp",
    "std::math::floor",
    "std::math::inf",
    "std::math::is_nan",
    "std::math::log",
    "std::math::nan",
    "std::math::pow",
    "std::math::sin",
    "std::math::sqrt",
    "std::math::tan",
    "std::math::tanh",
    "std::process::pid",
    "std::str::builder_append",
    "std::str::builder_finish",
    "std::str::builder_len",
    "std::str::builder_new",
    "std::str::can_parse_float",
    "std::str::can_parse_int",
    "std::str::clone",
    "std::str::from_bytes",
    "std::str::index_of",
    "std::str::lower",
    "std::str::pad_left",
    "std::str::pad_right",
    "std::str::repeat",
    "std::str::replace",
    "std::str::substring",
    "std::str::trim",
    "std::str::upper",
    "std::text::is_alnum",
    "std::text::is_alpha",
    "std::text::is_digit",
    "std::text::is_whitespace",
    "std::text::is_word_char",
    "std::text::tokenize_words_into",
    "std::time::monotonic",
    "std::time::sleep",
];

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

/// No pair reaches "not implemented" (the third kind the plan allowed
/// for): every arm the scrape finds lowers or refuses by its own arm.
///
/// NOT ONE OF THE PLAN'S KINDS, so listed apart (S0's stop rule): the
/// bare arms of paths whose signature row is fallible. The checker
/// refuses a bare call of a fallible signature (GH #738: "`..` can fail
/// (..) and this call says nothing about it"), so these expression arms
/// are dead for every checked program, and `hale build` checks first.
/// They are the plan's "9 fallible rows that keep a dead bare arm",
/// which S5 removes; until then the IR shadow cannot reach them. Each
/// had a statement twin too, 18 dead arms in all; S1 folded the twins
/// into the fall-through, so a bare statement call of one reaches the
/// same dead expression arm. The test derives the list from the rows
/// and holds it equal.
const DEAD_BARE_OF_FALLIBLE_ROWS: &[&str] = &[
    "std::bytes::at",
    "std::io::fs::file_size",
    "std::io::fs::list_dir_at",
    "std::io::fs::list_dir_count",
    "std::io::fs::mkdir",
    "std::io::fs::read_bytes",
    "std::io::fs::read_file",
    "std::io::fs::write_file",
    "std::io::fs::write_file_append",
];

/// Every allowed pair, with the kind it is allowed as.
fn allowances() -> BTreeMap<(String, Position), &'static str> {
    let mut out = BTreeMap::new();
    for p in REFUSED_BARE {
        out.insert((p.to_string(), Position::Statement), "refused by lowering");
        out.insert((p.to_string(), Position::Expression), "refused by lowering");
    }
    for p in REFUSED_BARE_EXPRESSION_ONLY {
        out.insert((p.to_string(), Position::Expression), "refused by lowering");
    }
    for p in REFUSED_UNDER_OR {
        out.insert((p.to_string(), Position::Fallible), "refused by lowering");
    }
    for (p, position, _) in INTERNAL {
        out.insert((p.to_string(), *position), "internal");
    }
    for p in DEAD_BARE_OF_FALLIBLE_ROWS {
        out.insert((p.to_string(), Position::Expression), "dead bare arm of a fallible row");
    }
    out
}

/// The allowance lists say what the code says: the refused pairs are
/// exactly the refusal arms' pairs; each internal path is refused to a
/// user's program and called, at its position, by the stdlib
/// declarations named, all lowered in every build; the dead bare arms
/// are exactly the lowering arms at a bare position of a path whose
/// signature row is fallible.
#[test]
fn the_allowances_are_what_the_code_says() {
    let all = pairs(&scrape());
    let allowed = allowances();
    let refused_scraped: BTreeSet<(String, Position)> =
        all.iter().filter(|(_, k)| k.0 == ArmKind::Refuses).map(|(p, _)| p.clone()).collect();
    let refused_listed: BTreeSet<(String, Position)> =
        allowed.iter().filter(|(_, why)| **why == "refused by lowering").map(|(p, _)| p.clone()).collect();
    assert_eq!(refused_listed, refused_scraped, "the refusal allowances drifted from the refusal arms");

    let dead_derived: BTreeSet<(String, Position)> = all
        .iter()
        .filter(|((path, position), k)| {
            k.0 == ArmKind::Lowers
                && *position != Position::Fallible
                && hale_types::stdlib_surface::signature_for(&path.split("::").collect::<Vec<_>>())
                    .is_some_and(|s| s.fallible.is_some())
        })
        .map(|(p, _)| p.clone())
        .collect();
    let dead_listed: BTreeSet<(String, Position)> = allowed
        .iter()
        .filter(|(_, why)| **why == "dead bare arm of a fallible row")
        .map(|(p, _)| p.clone())
        .collect();
    assert_eq!(dead_listed, dead_derived, "the dead bare arms drifted from the signature rows");

    let seeds = stdlib_calls();
    let ir = harness::build_source_ir_text(
        "fn main() { }\n",
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
    let mut by_literal: BTreeMap<String, BTreeSet<Position>> = BTreeMap::new();
    for s in &scraped {
        let literal: BTreeSet<String> = s
            .arms
            .iter()
            .flat_map(|a| a.patterns.iter())
            .filter(|p| p.iter().all(|seg| matches!(seg, Seg::Lit(_))))
            .map(|p| p.iter().map(|seg| if let Seg::Lit(l) = seg { l.as_str() } else { "" }).collect::<Vec<_>>().join("::"))
            .collect();
        eprintln!(
            "{:<11} {:>3} patterns ({} family) in {:>3} arms; {:>3} literal paths, {:>3} with families expanded ({} refused); {} shadowed patterns {:?}",
            s.position.word(),
            s.patterns,
            s.family_patterns,
            s.arms.len(),
            literal.len(),
            s.paths.len(),
            s.paths.values().filter(|k| k.0 == ArmKind::Refuses).count(),
            s.shadowed.len(),
            s.shadowed,
        );
        for p in s.paths.keys() {
            by_path.entry(p.clone()).or_default().insert(s.position);
        }
        for p in literal {
            by_literal.entry(p).or_default().insert(s.position);
        }
    }
    for (what, m) in [("literal", &by_literal), ("expanded", &by_path)] {
        let multi = m.values().filter(|ps| ps.len() >= 2).count();
        let all3 = m.values().filter(|ps| ps.len() == 3).count();
        eprintln!("{what}: union {} paths; {} in two or more dispatchers ({} in all three)", m.len(), multi, all3);
    }
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
