//! An element stored through a parameter's place is kept in the caller's
//! frame (GH #1608 follow-up).
//!
//! A `push` or `set` of a pointer-shaped element (a `String`, `Bytes`, a
//! record or a locus) into a bounded place under a parameter
//! (`fn fill(r: Row) { push(r.xs, s) }`) anchors the element in the arena
//! the call allocates in. A free fn that does this is never scratch-local
//! (it takes a record), so that arena is its caller's current one: sound
//! when the caller hands a place of its own frame, and a use-after-free
//! when it hands a place a locus holds (`fill(self.row)`), whose elements
//! the frame frees under the field. Inside a locus member the arena is the
//! member's own scratch, which is wrong whatever the caller handed; the
//! checker refuses that at the intrinsic (`check.rs`,
//! `refuse_element_through_param`).
//!
//! This pass decides the free fns' half at the call. Over the program and
//! the stdlib's own bodies, a free fn *writes through* parameter `i` when
//! its body stores such an element into a place under it, directly or
//! through a bounded local viewing one, or hands a place under it to a fn
//! that writes through the parameter it lands in (a fixpoint). A call to
//! such a fn in the program is refused when the argument at `i` is a
//! place the frame does not own: one under `self`, a bounded local viewing
//! one, or, in a locus member, a place under one of its parameters. A
//! place under a free fn's own parameter passes the question to that fn's
//! callers, and a frame-local value (a `let` record, a temporary) is
//! sound. The stdlib's `std::http::is_route` is such a fn: it writes its
//! captures into `ctx.params`.
//!
//! Syntactic over declared types, once per check: nothing at run time.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{flat_decls, Block, Expr, LocusMember, Program, Stmt, TopDecl, TypeDeclBody, TypeExpr};
use hale_syntax::error::Diag;

use crate::listener_window::{visit_block, Visited};
use crate::symbol::Bundle;

/// Every call in the program's bodies that hands a free fn writing
/// through a parameter a place the frame does not own.
pub(crate) fn param_place_write_diags(bundle: &Bundle<'_>) -> Vec<Diag> {
    let mut programs: Vec<&Program> = bundle.programs.values().copied().collect();
    let user = programs.len();
    if let Some(std_prog) = crate::stdlib_bodies::program() {
        programs.push(std_prog);
    }
    let world = World::gather(&programs, &bundle.import_renames);
    let writers = world.writers();
    if writers.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for p in &programs[..user] {
        for d in flat_decls(&p.items) {
            let params = |ps: &'_ [hale_syntax::ast::Param]| -> BTreeMap<String, TypeExpr> {
                ps.iter().map(|p| (p.name.name.clone(), p.ty.clone())).collect()
            };
            match d {
                TopDecl::Fn(f) => world.calls(&f.body, &params(&f.params), None, &writers, &mut out),
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        let (body, ps) = match m {
                            LocusMember::Fn(f) => (&f.body, params(&f.params)),
                            LocusMember::Lifecycle(lc) => (&lc.body, params(&lc.params)),
                            LocusMember::Mode(md) => (&md.body, params(&md.params)),
                            LocusMember::Failure(fd) => (&fd.body, params(&fd.params)),
                            _ => continue,
                        };
                        world.calls(body, &ps, Some(&l.name.name), &writers, &mut out);
                    }
                }
                _ => {}
            }
        }
    }
    out
}

struct FreeFn<'a> {
    params: Vec<(String, &'a TypeExpr)>,
    body: &'a Block,
}

struct World<'a> {
    /// Record and locus fields by declared name.
    fields: BTreeMap<String, BTreeMap<String, &'a TypeExpr>>,
    /// Which names are records (a pointer-shaped element) or loci.
    records: BTreeSet<String>,
    fns: BTreeMap<String, FreeFn<'a>>,
    renames: BTreeMap<String, String>,
}

/// The root of a place expression and its field path: `self.a.b` is
/// (`self`, [a, b]); `x[i].y` is (`x`, [y]).
fn place(e: &Expr) -> Option<(String, Vec<String>)> {
    match e {
        Expr::Ident(i) => Some((i.name.clone(), Vec::new())),
        Expr::KwSelf(_) => Some(("self".to_string(), Vec::new())),
        Expr::Field { receiver, name, .. } => {
            let (root, mut path) = place(receiver)?;
            path.push(name.name.clone());
            Some((root, path))
        }
        Expr::Index { receiver, .. } => place(receiver),
        _ => None,
    }
}

impl<'a> World<'a> {
    fn gather(programs: &[&'a Program], import_renames: &[(Vec<String>, String)]) -> World<'a> {
        let mut w = World { fields: BTreeMap::new(), records: BTreeSet::new(), fns: BTreeMap::new(), renames: BTreeMap::new() };
        for (path, name) in hale_stdlib::PATH_RENAMES {
            w.renames.insert(path.join("::"), name.to_string());
        }
        for (path, mangled) in import_renames {
            w.renames.insert(path.join("::"), mangled.clone());
        }
        for p in programs {
            for d in flat_decls(&p.items) {
                match d {
                    TopDecl::Type(t) => {
                        if let TypeDeclBody::Struct(fs) = &t.body {
                            w.records.insert(t.name.name.clone());
                            w.fields.insert(t.name.name.clone(), fs.iter().map(|f| (f.name.name.clone(), &f.ty)).collect());
                        }
                    }
                    TopDecl::Locus(l) => {
                        w.records.insert(l.name.name.clone());
                        let mut fs = BTreeMap::new();
                        for m in &l.members {
                            if let LocusMember::Params(pb) = m {
                                for p in &pb.params {
                                    if let Some(t) = &p.ty {
                                        fs.insert(p.name.name.clone(), t);
                                    }
                                }
                            }
                        }
                        w.fields.insert(l.name.name.clone(), fs);
                    }
                    TopDecl::Fn(f) if f.generics.is_empty() && f.ffi.is_none() => {
                        w.fns.insert(
                            f.name.name.clone(),
                            FreeFn { params: f.params.iter().map(|p| (p.name.name.clone(), &p.ty)).collect(), body: &f.body },
                        );
                    }
                    _ => {}
                }
            }
        }
        w
    }

    /// A declared name as the program has it: a stdlib or an import path
    /// renamed, else its last segment.
    fn name_of(&self, segs: &[String]) -> String {
        let joined = segs.join("::");
        self.renames.get(&joined).cloned().unwrap_or_else(|| segs.last().cloned().unwrap_or_default())
    }

    fn named(&self, te: &TypeExpr) -> Option<String> {
        match te {
            TypeExpr::Named { path, .. } => {
                Some(self.name_of(&path.segments.iter().map(|s| s.name.clone()).collect::<Vec<_>>()))
            }
            _ => None,
        }
    }

    /// The type at `path` under a value of type `te`.
    fn type_at(&self, te: &'a TypeExpr, path: &[String]) -> Option<&'a TypeExpr> {
        let mut t = te;
        for f in path {
            let n = self.named(t)?;
            t = self.fields.get(&n)?.get(f)?;
        }
        Some(t)
    }

    /// Whether `te` is a `bounded` whose element is pointer-shaped.
    fn pointer_bounded(&self, te: &TypeExpr) -> bool {
        let TypeExpr::Bounded { elem, .. } = te else { return false };
        match elem.as_ref() {
            TypeExpr::Primitive(hale_syntax::ast::PrimType::String | hale_syntax::ast::PrimType::Bytes, _) => true,
            other => self.named(other).is_some_and(|n| self.records.contains(&n)),
        }
    }

    fn callee(&self, e: &Expr) -> Option<String> {
        match e {
            Expr::Ident(i) => Some(i.name.clone()),
            Expr::Path(q) => Some(self.name_of(&q.segments.iter().map(|s| s.name.clone()).collect::<Vec<_>>())),
            _ => None,
        }
    }

    /// The free fns that write through a parameter, and which: a
    /// greatest-fixpoint closure over the bodies.
    fn writers(&self) -> BTreeMap<String, BTreeSet<usize>> {
        let mut out: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        loop {
            let mut changed = false;
            for (name, f) in &self.fns {
                for i in self.writes_of(f, &out) {
                    changed |= out.entry(name.clone()).or_default().insert(i);
                }
            }
            if !changed {
                return out;
            }
        }
    }

    /// The parameters `f`'s body writes through, given what is known of
    /// the fns it calls.
    fn writes_of(&self, f: &FreeFn<'a>, known: &BTreeMap<String, BTreeSet<usize>>) -> BTreeSet<usize> {
        let index: BTreeMap<&str, usize> = f.params.iter().enumerate().map(|(i, (n, _))| (n.as_str(), i)).collect();
        // a bounded local viewing a place under a parameter: the parameter
        let mut views: BTreeMap<String, (usize, Vec<String>)> = BTreeMap::new();
        let mut out = BTreeSet::new();
        let mut seen: Vec<(String, Vec<Expr>)> = Vec::new();
        visit_block(f.body, &mut |v| match v {
            Visited::Stmt(Stmt::Let { name, value, .. }) => {
                if let Some((root, path)) = place(value) {
                    if let Some(&i) = index.get(root.as_str()) {
                        views.insert(name.name.clone(), (i, path));
                    } else if let Some((i, p0)) = views.get(&root).cloned() {
                        views.insert(name.name.clone(), (i, p0.into_iter().chain(path).collect()));
                    }
                }
            }
            Visited::Expr(Expr::Call { callee, args, .. }) => {
                if let Some(c) = self.callee(callee) {
                    seen.push((c, args.clone()));
                }
            }
            _ => {}
        });
        let root_of = |e: &Expr| -> Option<(usize, Vec<String>)> {
            let (root, path) = place(e)?;
            if let Some(&i) = index.get(root.as_str()) {
                return Some((i, path));
            }
            let (i, p0) = views.get(&root)?.clone();
            Some((i, p0.into_iter().chain(path).collect()))
        };
        for (c, args) in &seen {
            if (c == "push" && args.len() == 2) || (c == "set" && args.len() == 3) {
                if let Some((i, path)) = root_of(&args[0]) {
                    if self.type_at(f.params[i].1, &path).is_some_and(|t| self.pointer_bounded(t)) {
                        out.insert(i);
                    }
                }
            } else if let Some(ks) = known.get(c) {
                for &k in ks {
                    if let Some((i, _)) = args.get(k).and_then(root_of) {
                        out.insert(i);
                    }
                }
            }
        }
        out
    }

    /// The type of the place `root.path` in a body whose parameters are
    /// `params` and whose `self` is `locus`.
    fn place_type(&self, root: &str, path: &[String], params: &BTreeMap<String, TypeExpr>, locus: Option<&str>) -> Option<TypeExpr> {
        if root == "self" {
            let mut path = path.iter();
            let first = path.next()?;
            let mut t: &TypeExpr = self.fields.get(locus?)?.get(first)?;
            for f in path {
                let n = self.named(t)?;
                t = self.fields.get(&n)?.get(f)?;
            }
            return Some(t.clone());
        }
        let te = params.get(root)?;
        let mut t: TypeExpr = te.clone();
        for f in path {
            let n = self.named(&t)?;
            t = (*self.fields.get(&n)?.get(f)?).clone();
        }
        Some(t)
    }

    /// The calls in `body` that hand a writer a place the frame does not
    /// own. `params` are the body's parameters; `locus` is set for a locus
    /// member, whose own parameters' places are not the frame's to write
    /// through either (its arena is its scratch).
    fn calls(
        &self,
        body: &Block,
        params: &BTreeMap<String, TypeExpr>,
        locus: Option<&str>,
        writers: &BTreeMap<String, BTreeSet<usize>>,
        out: &mut Vec<Diag>,
    ) {
        let member = locus.is_some();
        // a bounded local bound from a place is that place: its root
        let mut views: BTreeMap<String, String> = BTreeMap::new();
        let mut found: Vec<Diag> = Vec::new();
        visit_block(body, &mut |v| match v {
            Visited::Stmt(Stmt::Let { name, value, .. }) => {
                views.remove(&name.name);
                if let Some((root, path)) = place(value) {
                    if let Some(base) = views.get(&root).cloned() {
                        views.insert(name.name.clone(), base);
                    } else if self
                        .place_type(&root, &path, params, locus)
                        .is_some_and(|t| matches!(t, TypeExpr::Bounded { .. }))
                    {
                        views.insert(name.name.clone(), root);
                    }
                }
            }
            Visited::Expr(Expr::Call { callee, args, .. }) => {
                let Some(c) = self.callee(callee) else { return };
                let Some(ks) = writers.get(&c) else { return };
                for &k in ks {
                    let Some(arg) = args.get(k) else { continue };
                    let Some((root, _)) = place(arg) else { continue };
                    let base = views.get(&root).cloned().unwrap_or(root);
                    let foreign = base == "self" || (member && params.contains_key(&base));
                    if !foreign {
                        continue;
                    }
                    let shown = match callee.as_ref() {
                        Expr::Path(q) => q.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::"),
                        Expr::Ident(i) => i.name.clone(),
                        _ => c.clone(),
                    };
                    let whose = if base == "self" { "this locus's arena" } else { "its caller's frame, not this member's" };
                    found.push(Diag::ty(
                        arg.span(),
                        format!(
                            "`{shown}` stores pointer-shaped elements (a String, Bytes or a record) into a bounded \
                             place under its parameter, keeping them in the frame that calls it; this argument is a \
                             place in {whose}, so it would hold elements that frame frees. Pass a local and store it \
                             whole into the place after the call, or write the elements through `self` in a method of \
                             the locus that holds the place"
                        ),
                    ));
                }
            }
            _ => {}
        });
        out.extend(found);
    }
}
