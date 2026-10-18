//! GH #1530 step 2: a seam, expanded into ordinary methods.
//!
//! A seam is a chain of hops inside one locus tower: a source (or the
//! callable form's parameter), then `-> hop` after `-> hop`, a hop being a
//! method (`self.m`, `self.child.m`) or a locus (`self`, `self.child`),
//! where the compiler writes the dispatch: each member of the incoming set
//! goes to the method whose parameter takes it. This pass reads the hops'
//! declarations, checks coverage both ways, places every failure, and
//! writes the seam as the method it means (spec/semantics.md § Seams):
//!
//! - the closed flow (a source, and a last hop that returns nothing) is the
//!   locus's `run`: `while !self.draining { let v = <source> or { break; };
//!   <chain> }`;
//! - the callable form, `seam name(x: T) [-> R] { x -> … ; }`, is `fn
//!   name(x: T) -> R`, which returns the last hop's value.
//!
//! The chain is generated a code path per type: a locus hop meeting a set
//! is a `match` whose every arm runs the rest of the chain on that member,
//! and a hop meeting a member calls its eater directly. So no value of a
//! union nobody declared is ever built, and a member keeps its static type
//! from the producer's return to the sink. A failure is taken by the first
//! later hop whose parameter set holds it (entering the chain there), else
//! by the seam's `or end;` (the flow ends, as `run` ends; a callable seam
//! fails with it) or `or violate NAME;`; a failure placed nowhere is a
//! refusal.
//!
//! The generated source lives in its own parse space ([`SEAM_BASE`]), past
//! every file, and every hop call in it is one lowering inlines into the
//! seam, so the tag a producer's return site writes is the one the
//! dispatch reads a moment later, and optimization folds the branch
//! (`hale-codegen`'s seam inlining). A seam the pass refuses stays in the
//! program as written, and the checker reports it ([`refusals`]).

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{
    Expr, FnDecl, LifecycleKind, LocusDecl, LocusMember, ParamInit, PlacementSpec, Program, SeamDecl, SeamFailure,
    SeamHop, TopDecl, TypeDecl, TypeDeclBody, TypeExpr,
};
use hale_syntax::{Diag, Span};

/// The parse space of generated seams: past the api expansion's windows.
pub const SEAM_BASE: u32 = hale_syntax::api_names::API_SYNTH_BASE + 0x0C00_0000;

/// Each seam's window within [`SEAM_BASE`].
const SEAM_WINDOW: u32 = 0x0001_0000;

/// Whether an offset is in a generated seam (a hop call lowering inlines).
pub fn is_seam_pos(pos: u32) -> bool {
    (SEAM_BASE..SEAM_BASE + 0x0100_0000).contains(&pos)
}

/// The placeholder the generated closed flow calls where its source goes.
const SOURCE_PLACEHOLDER: &str = "__seam_source";

/// Expand every seam of the bundle into its methods, in place. A seam the
/// plan refuses stays where it is written, for [`refusals`].
pub fn expand(programs: &mut [&mut Program], context: &[&Program]) {
    let mut generated: Vec<(usize, String, Vec<LocusMember>)> = Vec::new();
    {
        let ro: Vec<&Program> = programs.iter().map(|p| &**p).chain(context.iter().copied()).collect();
        let index = Index::of(&ro);
        let mut n = 0u32;
        for (pi, p) in programs.iter().enumerate() {
            for l in loci(&p.items) {
                for m in &l.members {
                    let LocusMember::Seam(sd) = m else { continue };
                    let Ok(plan) = plan(&index, l, sd) else { continue };
                    let base = SEAM_BASE + n * SEAM_WINDOW;
                    n += 1;
                    let Some(members) = parse_members(&plan.text, base, &sd.head) else { continue };
                    generated.push((pi, l.name.name.clone(), members));
                }
            }
        }
    }
    for (pi, locus, members) in generated {
        let Some(l) = loci_mut(&mut programs[pi].items).into_iter().find(|l| l.name.name == locus) else { continue };
        // the seam is its methods now; every other seam of the locus
        // that was refused stays for the checker
        let mut left: Vec<LocusMember> = Vec::new();
        let mut expanded_one = false;
        for m in std::mem::take(&mut l.members) {
            match m {
                LocusMember::Seam(sd) if !expanded_one && is_planned(&sd, &members) => expanded_one = true,
                other => left.push(other),
            }
        }
        left.extend(members);
        l.members = left;
    }
}

/// Whether `members` is the expansion of `sd` (its method, or `run`).
fn is_planned(sd: &SeamDecl, members: &[LocusMember]) -> bool {
    members.iter().any(|m| match (m, &sd.name) {
        (LocusMember::Fn(f), Some(n)) => f.name.name == n.name,
        (LocusMember::Lifecycle(lc), None) => lc.kind == LifecycleKind::Run,
        _ => false,
    })
}

/// The refusals of every seam still in the bundle: the ones [`expand`]
/// left as written.
pub fn refusals(programs: &[&Program], context: &[&Program]) -> Vec<Diag> {
    let ro: Vec<&Program> = programs.iter().copied().chain(context.iter().copied()).collect();
    let index = Index::of(&ro);
    let mut out = Vec::new();
    for p in programs {
        for l in loci(&p.items) {
            for m in &l.members {
                if let LocusMember::Seam(sd) = m {
                    match plan(&index, l, sd) {
                        Err(ds) => out.extend(ds),
                        // planned but not expanded: its generated source
                        // did not parse, a compiler bug
                        Ok(_) => out.push(Diag::ty(
                            sd.span,
                            "the seam's generated methods did not parse (a compiler bug); write the chain as \
                             methods until it is fixed",
                        )),
                    }
                }
            }
        }
    }
    out
}

fn loci(items: &[TopDecl]) -> Vec<&LocusDecl> {
    let mut out = Vec::new();
    for it in items {
        match it {
            TopDecl::Locus(l) => out.push(l),
            TopDecl::Module(m) => out.extend(loci(&m.items)),
            _ => {}
        }
    }
    out
}

fn loci_mut(items: &mut [TopDecl]) -> Vec<&mut LocusDecl> {
    let mut out = Vec::new();
    for it in items {
        match it {
            TopDecl::Locus(l) => out.push(l),
            TopDecl::Module(m) => out.extend(loci_mut(&mut m.items)),
            _ => {}
        }
    }
    out
}

/// Parse the generated members inside a carrier locus, and put the
/// source's expression where the closed flow calls the placeholder.
fn parse_members(text: &str, base: u32, head: &Expr) -> Option<Vec<LocusMember>> {
    let program = hale_syntax::parse_source_at(&format!("locus __SeamCarrier {{\n{text}}}\n"), base).ok()?;
    let mut members = match program.items.into_iter().next()? {
        TopDecl::Locus(l) => l.members,
        _ => return None,
    };
    for m in &mut members {
        let body = match m {
            LocusMember::Fn(f) => &mut f.body,
            LocusMember::Lifecycle(lc) => &mut lc.body,
            _ => continue,
        };
        let placeholder = |e: &Expr| {
            matches!(e, Expr::Call { callee, args, .. }
                if args.is_empty() && matches!(callee.as_ref(), Expr::Ident(i) if i.name == SOURCE_PLACEHOLDER))
        };
        crate::rpc_expand::walk_block_mut(body, &mut |e| {
            // the source's `or` is the author's, not a hop: it takes the
            // source's span, so lowering inlines nothing of it
            if let Expr::Or { inner, span, .. } = e {
                if placeholder(inner) {
                    *span = head.span();
                }
            }
            if placeholder(e) {
                *e = head.clone();
            }
        });
    }
    Some(members)
}

// ---- the declarations a seam reads ----

/// The bundle's types and loci, by name.
struct Index<'a> {
    types: BTreeMap<String, &'a TypeDecl>,
    loci: BTreeMap<String, &'a LocusDecl>,
    interfaces: BTreeSet<String>,
}

impl<'a> Index<'a> {
    fn of(programs: &[&'a Program]) -> Index<'a> {
        let mut ix = Index { types: BTreeMap::new(), loci: BTreeMap::new(), interfaces: BTreeSet::new() };
        fn walk<'a>(items: &'a [TopDecl], ix: &mut Index<'a>) {
            for it in items {
                match it {
                    TopDecl::Type(t) => {
                        ix.types.entry(t.name.name.clone()).or_insert(t);
                    }
                    TopDecl::Locus(l) => {
                        ix.loci.entry(l.name.name.clone()).or_insert(l);
                        for m in &l.members {
                            if let LocusMember::Type(t) = m {
                                ix.types.entry(t.name.name.clone()).or_insert(t);
                            }
                        }
                    }
                    TopDecl::Interface(i) => {
                        ix.interfaces.insert(i.name.name.clone());
                    }
                    TopDecl::Module(m) => walk(&m.items, ix),
                    _ => {}
                }
            }
        }
        for p in programs {
            walk(&p.items, &mut ix);
        }
        ix
    }

    /// A type as the seam compares it: its spelling, an alias followed to
    /// what it names.
    fn key(&self, te: &TypeExpr) -> String {
        let mut te = te.clone();
        for _ in 0..16 {
            let next = match &te {
                TypeExpr::Named { path, generic_args, .. } if generic_args.is_empty() && path.segments.len() == 1 => {
                    match self.types.get(&path.segments[0].name).map(|t| &t.body) {
                        Some(TypeDeclBody::Alias(a)) => Some(a.clone()),
                        _ => None,
                    }
                }
                _ => None,
            };
            match next {
                Some(n) => te = n,
                None => break,
            }
        }
        crate::surfaces::type_text(&te)
    }

    /// The set `key` names, as its members' keys and variant names.
    fn set(&self, key: &str) -> Option<Vec<(String, String)>> {
        let t = self.types.get(key)?;
        if !t.is_set() {
            return None;
        }
        let TypeDeclBody::Enum(vs) = &t.body else { return None };
        Some(vs.iter().map(|v| (self.key(&v.fields[0]), v.name.name.clone())).collect())
    }

    /// The members a value of `key` may be: a set's members, or itself.
    fn members(&self, key: &str) -> Vec<String> {
        match self.set(key) {
            Some(ms) => ms.into_iter().map(|(k, _)| k).collect(),
            None => vec![key.to_string()],
        }
    }

    /// Whether a value of `got` flows into a parameter of `want`: the same
    /// type, a member of the set it is, or a set all of whose members are.
    fn covers(&self, want: &str, got: &str) -> bool {
        if want == got {
            return true;
        }
        let ms = self.members(want);
        self.members(got).iter().all(|g| ms.contains(g))
    }
}

/// A method of a hop: its receiver as the chain spells it, and its
/// declaration.
#[derive(Clone)]
struct Method<'a> {
    recv: String,
    decl: &'a FnDecl,
}

impl Method<'_> {
    fn label(&self) -> String {
        format!("{}.{}", self.recv, self.decl.name.name)
    }
}

/// A resolved hop.
enum Hop<'a> {
    /// One method, named.
    Method(Method<'a>),
    /// A locus: its methods of one parameter, among which a member finds
    /// its eater.
    Locus { label: String, methods: Vec<Method<'a>> },
}

/// What the plan of one seam is: the generated members' source.
struct Plan {
    text: String,
}

/// The owned child locus a `params` field of `owner` holds, by its type.
fn held_child<'a>(index: &Index<'a>, owner: &LocusDecl, field: &str) -> Option<Result<&'a LocusDecl, String>> {
    for m in &owner.members {
        let LocusMember::Params(pb) = m else { continue };
        for p in &pb.params {
            if p.name.name != field {
                continue;
            }
            let name = match (&p.ty, &p.init) {
                (Some(TypeExpr::Named { path, .. }), _) => path.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::"),
                (None, ParamInit::Value(Expr::Struct { path: name, .. })) => {
                    name.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::")
                }
                _ => return Some(Err(format!("`self.{field}` is no child locus whose type the seam can read"))),
            };
            if index.interfaces.contains(&name) {
                return Some(Err(format!(
                    "`self.{field}` is the interface `{name}`, so the method a hop on it calls is not known here: a \
                     seam's chain is static"
                )));
            }
            return Some(match index.loci.get(&name) {
                Some(l) => Ok(*l),
                None => Err(format!("`self.{field}` is a `{name}`, not a locus: a hop is a method or a child locus")),
            });
        }
    }
    None
}

/// A main locus's child placed on a pool other than main's is in another
/// tower: the bus crosses pools, a seam does not.
fn placed_elsewhere(owner: &LocusDecl, field: &str) -> Option<String> {
    if !owner.is_main {
        return None;
    }
    for m in &owner.members {
        let LocusMember::Placement(pb) = m else { continue };
        for e in &pb.entries {
            if e.field.name != field {
                continue;
            }
            return match &e.spec {
                PlacementSpec::Cooperative { pool: None, .. } => None,
                PlacementSpec::Cooperative { pool: Some(p), .. } if p.name == "main" => None,
                PlacementSpec::Cooperative { pool: Some(p), .. } => Some(format!("pool `{}`", p.name)),
                PlacementSpec::Pinned { .. } => Some("a pinned thread".to_string()),
            };
        }
    }
    None
}

fn fns(l: &LocusDecl) -> impl Iterator<Item = &FnDecl> {
    l.members.iter().filter_map(|m| match m {
        LocusMember::Fn(f) => Some(f),
        _ => None,
    })
}

/// Resolve one hop against the seam's locus.
fn resolve<'a>(index: &Index<'a>, owner: &'a LocusDecl, hop: &SeamHop) -> Result<Hop<'a>, Diag> {
    let names: Vec<&str> = hop.path.iter().map(|i| i.name.as_str()).collect();
    let refuse = |m: String| Err(Diag::ty(hop.span, m));
    let locus_hop = |label: String, l: &'a LocusDecl, recv: String| Hop::Locus {
        label,
        methods: fns(l)
            .filter(|f| f.params.len() == 1 && f.generics.is_empty())
            .map(|f| Method { recv: recv.clone(), decl: f })
            .collect(),
    };
    match names.as_slice() {
        [] => Ok(locus_hop("self".to_string(), owner, "self".to_string())),
        [x] => match held_child(index, owner, x) {
            Some(Ok(l)) => {
                if let Some(at) = placed_elsewhere(owner, x) {
                    return refuse(format!(
                        "`self.{x}` runs on {at}, another tower than the seam's: a seam is one tower; across pools, \
                         use the bus"
                    ));
                }
                Ok(locus_hop(format!("self.{x}"), l, format!("self.{x}")))
            }
            Some(Err(why)) => refuse(why),
            None => match fns(owner).find(|f| f.name.name == *x) {
                Some(f) => Ok(Hop::Method(Method { recv: "self".to_string(), decl: f })),
                None => refuse(format!("`self.{x}` is neither a method of `{}` nor a child locus of it", owner.name.name)),
            },
        },
        [x, m] => match held_child(index, owner, x) {
            Some(Ok(l)) => {
                if let Some(at) = placed_elsewhere(owner, x) {
                    return refuse(format!(
                        "`self.{x}` runs on {at}, another tower than the seam's: a seam is one tower; across pools, \
                         use the bus"
                    ));
                }
                match fns(l).find(|f| f.name.name == *m) {
                    Some(f) => Ok(Hop::Method(Method { recv: format!("self.{x}"), decl: f })),
                    None => refuse(format!("`{}` has no method `{m}`", l.name.name)),
                }
            }
            Some(Err(why)) => refuse(why),
            None => refuse(format!("`self.{x}` is not a child locus of `{}`", owner.name.name)),
        },
        _ => refuse("a hop is `self`, `self.child`, `self.method` or `self.child.method`".to_string()),
    }
}

/// The method a call in the chain names, when the chain's head is one
/// (`self.m(…)`, `self.child.m(…)`).
fn head_method<'a>(index: &Index<'a>, owner: &'a LocusDecl, head: &Expr) -> Option<&'a FnDecl> {
    let Expr::Call { callee, .. } = head else { return None };
    let Expr::Field { receiver, name, .. } = callee.as_ref() else { return None };
    match receiver.as_ref() {
        Expr::KwSelf(_) => fns(owner).find(|f| f.name.name == name.name),
        Expr::Field { receiver: r, name: field, .. } if matches!(r.as_ref(), Expr::KwSelf(_)) => {
            match held_child(index, owner, &field.name)? {
                Ok(l) => fns(l).find(|f| f.name.name == name.name),
                Err(_) => None,
            }
        }
        _ => None,
    }
}

struct Gen<'p, 'a> {
    index: &'p Index<'a>,
    hops: &'p [Hop<'a>],
    spans: Vec<Span>,
    seam: &'p SeamDecl,
    closed: bool,
    errors: Vec<Diag>,
    fresh: usize,
    /// The callable form's result types, each path's.
    ends: BTreeSet<String>,
    /// The failures the callable form fails with (`or end;`).
    fails: BTreeSet<String>,
    /// A callable seam's failure routes: each the failure's type and the
    /// statements that run the rest of the chain on it, a method of its own.
    helpers: Vec<(String, String)>,
}

/// Where a callable seam's failure route calls its method, patched once the
/// seam's result and failure types are known.
const HELPER_CALL: &str = "@@seam_helper:";

impl Gen<'_, '_> {
    fn var(&mut self) -> String {
        self.fresh += 1;
        format!("__seam_v{}", self.fresh)
    }

    fn refuse(&mut self, span: Span, m: String) {
        if !self.errors.iter().any(|d| d.message == m) {
            self.errors.push(Diag::ty(span, m));
        }
    }

    /// The statements that run the chain from hop `k` on `var`, a value of
    /// `ty`; `from` names what produced it.
    fn chain(&mut self, k: usize, var: &str, ty: &str, from: &str, out: &mut String) {
        if k == self.hops.len() {
            self.end(var, ty, from, out);
            return;
        }
        let span = self.spans[k];
        match &self.hops[k] {
            Hop::Method(m) => {
                let m = m.clone();
                let want = self.index.key(&m.decl.params.first().map(|p| p.ty.clone()).unwrap_or_else(unit));
                if m.decl.params.len() != 1 {
                    self.refuse(span, format!("`{}` takes {} parameters; a hop takes one, the value", m.label(), m.decl.params.len()));
                    return;
                }
                if self.index.members(ty).iter().all(|g| self.index.covers(&want, g)) {
                    // the same set, or a member, flows in as it is; another
                    // set, each of its members on its own
                    if want == ty || self.index.set(ty).is_none() {
                        self.call(k, &m, var, out);
                    } else {
                        // a set into a wider set: each member on its own
                        self.split(k, var, ty, out, |g, k, x, _member, out| {
                            g.call(k, &m, x, out);
                        });
                    }
                } else {
                    for got in self.index.members(ty) {
                        if !self.index.covers(&want, &got) {
                            let label = m.label();
                            self.refuse(
                                span,
                                format!("`{label}` takes `{want}`, and `{got}` reaching it from {from} is not one of its members"),
                            );
                        }
                    }
                }
            }
            Hop::Locus { label, methods } => {
                let (label, methods) = (label.clone(), methods.clone());
                let mut eaters: Vec<(String, Method)> = Vec::new();
                for got in self.index.members(ty) {
                    let takes: Vec<&Method> = methods
                        .iter()
                        .filter(|m| self.index.members(&self.index.key(&m.decl.params[0].ty)).contains(&got))
                        .collect();
                    match takes.as_slice() {
                        [one] => eaters.push((got, (*one).clone())),
                        [] => self.refuse(
                            span,
                            format!("`{label}` has no method taking `{got}`, which reaches it from {from}: every member has one eater"),
                        ),
                        [a, b, ..] => self.refuse(
                            span,
                            format!(
                                "`{}` and `{}` both take `{got}`: a member has one eater",
                                a.label(),
                                b.label()
                            ),
                        ),
                    }
                }
                if self.errors.iter().any(|d| d.span == span) {
                    return;
                }
                if self.index.set(ty).is_none() {
                    let (_, m) = eaters.remove(0);
                    self.call(k, &m, var, out);
                } else {
                    self.split(k, var, ty, out, |g, k, x, member, out| {
                        let m = eaters.iter().find(|(got, _)| got == member).map(|(_, m)| m.clone()).expect("an eater");
                        g.call(k, &m, x, out);
                    });
                }
            }
        }
    }

    /// A `match` on `var`, a value of the set `ty`, whose arm for each
    /// member runs `each` on the member's value.
    fn split(
        &mut self,
        k: usize,
        var: &str,
        ty: &str,
        out: &mut String,
        mut each: impl FnMut(&mut Self, usize, &str, &str, &mut String),
    ) {
        let members = self.index.set(ty).unwrap_or_default();
        out.push_str(&format!("match {var} {{\n"));
        for (member, variant) in members {
            let x = self.var();
            out.push_str(&format!("{variant}({x}) -> {{\n"));
            each(self, k, &x, &member, out);
            out.push_str("},\n");
        }
        out.push_str("}\n");
    }

    /// The call of `m` at hop `k` on `arg`, its failure placed, and the
    /// rest of the chain on its result.
    fn call(&mut self, k: usize, m: &Method, arg: &str, out: &mut String) {
        let label = m.label();
        let call = format!("{}.{}({arg})", m.recv, m.decl.name.name);
        // `-> ()` is no result, as it is everywhere (the desugar sequence
        // normalizes it after this pass)
        let ret = m.decl.ret.as_ref().filter(|t| !matches!(t, TypeExpr::Tuple(ts, _) if ts.is_empty())).map(|t| self.index.key(t));
        let v = self.var();
        let failure = match &m.decl.fallible {
            Some(e) => {
                let e = self.index.key(e);
                Some(self.failure(k, &e, &label))
            }
            None => None,
        };
        let disposition = failure.map(|f| format!(" or {{\n{f}}}")).unwrap_or_default();
        match ret {
            Some(ret) => {
                let mut rest = String::new();
                self.chain(k + 1, &v, &ret, &format!("`{label}`"), &mut rest);
                // a set split at once, and used nowhere else, is a fused
                // binding (`__seam_sN`): lowering hands it back in the
                // seam's frame (`hale-codegen`'s seam fusion)
                let head = format!("match {v} {{\n");
                let uses = rest.match_indices(v.as_str()).filter(|(i, _)| {
                    !rest[i + v.len()..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
                });
                let (v, rest) = if rest.starts_with(&head) && uses.count() == 1 {
                    let s = v.replacen("__seam_v", "__seam_s", 1);
                    let rest = format!("match {s} {{\n{}", &rest[head.len()..]);
                    (s, rest)
                } else {
                    (v, rest)
                };
                out.push_str(&format!("let {v} = {call}{disposition};\n"));
                out.push_str(&rest);
            }
            None => {
                out.push_str(&format!("{call}{disposition};\n"));
                if k + 1 < self.hops.len() {
                    let span = self.spans[k + 1];
                    self.refuse(span, format!("`{label}` returns nothing, so the chain cannot go on past it"));
                } else if !self.closed {
                    self.ends.insert("()".to_string());
                    out.push_str("return;\n");
                }
            }
        }
    }

    /// The statements a failure of `e` at hop `k` runs.
    fn failure(&mut self, k: usize, e: &str, label: &str) -> String {
        for j in k + 1..self.hops.len() {
            let takes = match &self.hops[j] {
                Hop::Method(m) => m.decl.params.len() == 1 && self.index.covers(&self.index.key(&m.decl.params[0].ty), e),
                // every member of the failure (a set's, or itself) has an
                // eater there
                Hop::Locus { methods, .. } => self.index.members(e).iter().all(|got| {
                    methods.iter().any(|m| self.index.members(&self.index.key(&m.decl.params[0].ty)).contains(got))
                }),
            };
            if takes {
                let mut s = String::new();
                self.chain(j, "err", e, &format!("the failure of `{label}`"), &mut s);
                if self.closed {
                    s.push_str("continue;\n");
                    return s;
                }
                // a callable seam's route is a method of its own, whose
                // every path returns, so the `or` that calls it diverges
                // however the route branches
                let i = self.helpers.len();
                self.helpers.push((e.to_string(), s));
                return format!("{HELPER_CALL}{i}@@\n");
            }
        }
        match &self.seam.on_failure {
            Some(SeamFailure::End(_)) if self.closed => "break;\n".to_string(),
            Some(SeamFailure::End(_)) => {
                self.fails.insert(e.to_string());
                "fail err;\n".to_string()
            }
            Some(SeamFailure::Violate(c)) if self.closed => format!("violate {};\ncontinue;\n", c.name),
            Some(SeamFailure::Violate(c)) => {
                self.refuse(
                    c.span,
                    "a callable seam's uncovered failure fails the call: write `or end;` (a closure is violated by \
                     the flow `run` is, the closed seam)"
                        .to_string(),
                );
                String::new()
            }
            None => {
                let span = self.spans[k];
                self.refuse(
                    span,
                    format!(
                        "`{label}` fails with `{e}`, and no later hop takes it: add `{e}` to a later hop's set, or route \
                         it with `or end;` or `or violate <closure>;`"
                    ),
                );
                String::new()
            }
        }
    }

    /// The end of one path: the closed flow ended at its sink; the
    /// callable form returns the value.
    fn end(&mut self, var: &str, ty: &str, from: &str, out: &mut String) {
        if self.closed {
            let span = *self.spans.last().expect("a hop");
            self.refuse(
                span,
                format!(
                    "the seam's last hop ends with `{ty}` from {from}; a closed flow ends at a sink, a method that \
                     takes the value and returns nothing"
                ),
            );
            return;
        }
        if let Some(r) = declared_result(self.seam) {
            let r = self.index.key(r);
            if !self.index.covers(&r, ty) {
                let span = *self.spans.last().expect("a hop");
                self.refuse(span, format!("the seam returns `{r}`, and `{ty}` from {from} is not one of its members"));
            }
        }
        self.ends.insert(ty.to_string());
        out.push_str(&format!("return {var};\n"));
    }
}

/// A callable seam's declared result, `-> ()` being none, as everywhere
/// (the unit-return normalization runs after this pass).
fn declared_result(sd: &SeamDecl) -> Option<&TypeExpr> {
    sd.ret.as_ref().filter(|t| !matches!(t, TypeExpr::Tuple(ts, _) if ts.is_empty()))
}

fn unit() -> TypeExpr {
    TypeExpr::Tuple(Vec::new(), Span::new(0, 0))
}

/// The plan of the seam `sd` of the locus `owner`: its generated methods'
/// source, or every refusal.
fn plan(index: &Index<'_>, owner: &LocusDecl, sd: &SeamDecl) -> Result<Plan, Vec<Diag>> {
    let mut errors = Vec::new();
    let mut hops = Vec::new();
    for h in &sd.hops {
        match resolve(index, owner, h) {
            Ok(r) => hops.push(r),
            Err(d) => errors.push(d),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let closed = sd.param.is_none();
    if closed {
        let other_run = owner
            .members
            .iter()
            .any(|m| matches!(m, LocusMember::Lifecycle(lc) if lc.kind == LifecycleKind::Run && !lc.synthesized));
        let other_seam = owner.members.iter().filter(|m| matches!(m, LocusMember::Seam(s) if s.param.is_none())).count() > 1;
        if other_run || other_seam {
            errors.push(Diag::ty(
                sd.span,
                format!(
                    "`{}` has its own `run`{}: a closed seam is the locus's `run`, so the locus has one or the other",
                    owner.name.name,
                    if other_seam { " (another closed seam)" } else { "" }
                ),
            ));
            return Err(errors);
        }
    } else if let Some(n) = &sd.name {
        if fns(owner).any(|f| f.name.name == n.name) {
            errors.push(Diag::ty(n.span, format!("`{}` already has a method `{}`; the seam is one", owner.name.name, n.name)));
            return Err(errors);
        }
    }
    let mut g = Gen {
        index,
        hops: &hops,
        spans: sd.hops.iter().map(|h| h.span).collect(),
        seam: sd,
        closed,
        errors: Vec::new(),
        fresh: 0,
        ends: BTreeSet::new(),
        fails: BTreeSet::new(),
        helpers: Vec::new(),
    };
    let mut body = String::new();
    let text = if closed {
        let Some(src) = head_method(index, owner, &sd.head) else {
            return Err(vec![Diag::ty(
                sd.head.span(),
                "a closed seam's source is a call of a method of the locus or of a child locus (`self.feed.next()`), \
                 whose declaration says what it makes",
            )]);
        };
        let Some(entry) = src.ret.as_ref().map(|t| index.key(t)) else {
            return Err(vec![Diag::ty(sd.head.span(), format!("the seam's source `{}` returns nothing", src.name.name))]);
        };
        let or = if src.fallible.is_some() { " or {\nbreak;\n}" } else { "" };
        g.chain(0, "__seam_v0", &entry, &format!("the source `{}`", src.name.name), &mut body);
        format!("run() {{\nwhile !self.draining {{\nlet __seam_v0 = {SOURCE_PLACEHOLDER}(){or};\n{body}}}\n}}\n")
    } else {
        let p = sd.param.as_ref().expect("a callable seam's parameter");
        // the callable form's chain begins at its parameter, as written
        if !matches!(&sd.head, Expr::Ident(i) if i.name == p.name.name) {
            return Err(vec![Diag::ty(
                sd.head.span(),
                format!(
                    "a callable seam's chain begins at its parameter (`{} -> …`); a call there is a source, which \
                     only the closed seam (`seam {{ … }}`) has",
                    p.name.name
                ),
            )]);
        }
        let entry = index.key(&p.ty);
        g.chain(0, "__seam_v0", &entry, &format!("the parameter `{}`", p.name.name), &mut body);
        let ret = match declared_result(sd) {
            Some(r) => format!(" -> {}", index.key(r)),
            None => {
                let ends: Vec<&String> = g.ends.iter().collect();
                match ends.as_slice() {
                    [] => String::new(),
                    [one] if one.as_str() == "()" => String::new(),
                    [one] => format!(" -> {one}"),
                    many => {
                        let span = sd.name.as_ref().map_or(sd.span, |n| n.span);
                        g.errors.push(Diag::ty(
                            span,
                            format!(
                                "the seam's paths end in {}; declare what it returns (`seam {}(…) -> R`), a set holding \
                                 them all",
                                many.iter().map(|t| format!("`{t}`")).collect::<Vec<_>>().join(", "),
                                sd.name.as_ref().map_or("name", |n| n.name.as_str())
                            ),
                        ));
                        String::new()
                    }
                }
            }
        };
        let fails: Vec<&String> = g.fails.iter().collect();
        let fallible = match fails.as_slice() {
            [] => String::new(),
            [one] => format!(" fallible({one})"),
            many => {
                g.errors.push(Diag::ty(
                    sd.span,
                    format!(
                        "the seam fails with {}, and a callable seam fails with one type: route the others to a later \
                         hop",
                        many.iter().map(|t| format!("`{t}`")).collect::<Vec<_>>().join(", ")
                    ),
                ));
                String::new()
            }
        };
        let name = sd.name.as_ref().map_or("seam", |n| n.name.as_str());
        // each failure route, a method of its own; the `or` that takes the
        // failure returns what the route returns
        let or_raise = if fallible.is_empty() { "" } else { " or raise" };
        let route_call = |i: usize| {
            if ret.is_empty() {
                format!("self.__seam_{name}_f{i}(err){or_raise};\nreturn;")
            } else {
                format!("return self.__seam_{name}_f{i}(err){or_raise};")
            }
        };
        let patch = |text: &str| {
            let mut out = text.to_string();
            for i in 0..g.helpers.len() {
                out = out.replace(&format!("{HELPER_CALL}{i}@@"), &route_call(i));
            }
            out
        };
        let mut text = format!(
            "fn {name}({}: {}){ret}{fallible} {{\nlet __seam_v0 = {};\n{}}}\n",
            p.name.name,
            index.key(&p.ty),
            p.name.name,
            patch(&body)
        );
        for (i, (e, route)) in g.helpers.iter().enumerate() {
            text.push_str(&format!("fn __seam_{name}_f{i}(err: {e}){ret}{fallible} {{\n{}}}\n", patch(route)));
        }
        text
    };
    if !g.errors.is_empty() {
        return Err(g.errors);
    }
    Ok(Plan { text })
}
