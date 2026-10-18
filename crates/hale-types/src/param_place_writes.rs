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
//! Over the checker's types (`TopScope`, aliases resolved), once per
//! check: nothing at run time. A bounded local is a view only when its
//! type is a `bounded` (a `let` of a record copies it), and a call through
//! a function value names the fn the value is bound to (`let f = fill;`);
//! one whose fn the walk cannot name (a parameter of fn type, a computed
//! callee) is taken as a writer of every argument whose type holds such a
//! place: refused rather than guessed.

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{flat_decls, Block, Expr, LocusMember, PrimType, Program, Stmt, TopDecl, TypeExpr};
use hale_syntax::error::Diag;

use crate::listener_window::{visit_block, Visited};
use crate::resolve::TopScope;
use crate::symbol::{Bundle, TopSymbol, TypeKind};
use crate::ty::Ty;

/// Every call in the program's bodies that hands a free fn writing
/// through a parameter a place the frame does not own.
pub(crate) fn param_place_write_diags(bundle: &Bundle<'_>, top: &TopScope) -> Vec<Diag> {
    let mut programs: Vec<&Program> = bundle.programs.values().copied().collect();
    let user = programs.len();
    if let Some(std_prog) = crate::stdlib_bodies::program() {
        programs.push(std_prog);
    }
    let world = World::gather(&programs, &bundle.import_renames, top);
    let writers = world.writers();
    let mut out = Vec::new();
    for p in &programs[..user] {
        for d in flat_decls(&p.items) {
            let params = |ps: &[hale_syntax::ast::Param]| -> BTreeMap<String, Ty> {
                ps.iter().map(|p| (p.name.name.clone(), world.ty(&p.ty))).collect()
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
    params: Vec<(String, Ty)>,
    body: &'a Block,
}

struct World<'a, 't> {
    top: &'t TopScope,
    fns: BTreeMap<String, FreeFn<'a>>,
    renames: BTreeMap<String, String>,
}

/// The root of a place expression and its field path: `self.a.b` is
/// (`self`, [a, b]); `x[i].y` is (`x`, [y]), an index stepping into an
/// element (`Index` in the path).
fn place(e: &Expr) -> Option<(String, Vec<Step>)> {
    match e {
        Expr::Ident(i) => Some((i.name.clone(), Vec::new())),
        Expr::KwSelf(_) => Some(("self".to_string(), Vec::new())),
        Expr::Field { receiver, name, .. } => {
            let (root, mut path) = place(receiver)?;
            path.push(Step::Field(name.name.clone()));
            Some((root, path))
        }
        Expr::Index { receiver, .. } => {
            let (root, mut path) = place(receiver)?;
            path.push(Step::Index);
            Some((root, path))
        }
        _ => None,
    }
}

#[derive(Clone)]
enum Step {
    Field(String),
    Index,
}

/// How a body reaches the bounded places of its roots.
#[derive(Default)]
struct Bindings {
    /// A bounded local bound from a place: its root and the place's path.
    views: BTreeMap<String, (String, Vec<Step>)>,
    /// A local bound to a free fn's name.
    fn_values: BTreeMap<String, String>,
}

impl<'a, 't> World<'a, 't> {
    fn gather(programs: &[&'a Program], import_renames: &[(Vec<String>, String)], top: &'t TopScope) -> World<'a, 't> {
        let mut w = World { top, fns: BTreeMap::new(), renames: BTreeMap::new() };
        for (path, name) in hale_stdlib::PATH_RENAMES {
            w.renames.insert(path.join("::"), name.to_string());
        }
        for (path, mangled) in import_renames {
            w.renames.insert(path.join("::"), mangled.clone());
        }
        for p in programs {
            for d in flat_decls(&p.items) {
                if let TopDecl::Fn(f) = d {
                    if f.generics.is_empty() && f.ffi.is_none() {
                        let params = f.params.iter().map(|p| (p.name.name.clone(), w.ty(&p.ty))).collect();
                        w.fns.insert(f.name.name.clone(), FreeFn { params, body: &f.body });
                    }
                }
            }
        }
        w
    }

    fn ty(&self, te: &TypeExpr) -> Ty {
        crate::resolve::resolve_type_expr(te, &self.top.names)
    }

    /// `t` with its aliases resolved.
    fn dealias(&self, t: &Ty) -> Ty {
        let mut t = t.clone();
        for _ in 0..16 {
            match &t {
                Ty::Named(n) => match self.top.lookup(n) {
                    Some(TopSymbol::Type(info)) => match &info.kind {
                        TypeKind::Alias(a) => t = a.clone(),
                        _ => return t,
                    },
                    _ => return t,
                },
                _ => return t,
            }
        }
        t
    }

    /// The type of a step below a value of type `t`.
    fn step(&self, t: &Ty, s: &Step) -> Option<Ty> {
        match (self.dealias(t), s) {
            (Ty::Named(n), Step::Field(f)) => match self.top.lookup(&n) {
                Some(TopSymbol::Type(info)) => match &info.kind {
                    TypeKind::Struct(fields) => fields.iter().find(|x| &x.name == f).map(|x| x.ty.clone()),
                    _ => None,
                },
                Some(TopSymbol::Locus(li)) => li.params.iter().find(|p| &p.name == f).map(|p| p.ty.clone()),
                _ => None,
            },
            (Ty::Array(e, _), Step::Index) => Some(*e),
            _ => None,
        }
    }

    fn type_at(&self, t: &Ty, path: &[Step]) -> Option<Ty> {
        let mut t = t.clone();
        for s in path {
            t = self.step(&t, s)?;
        }
        Some(self.dealias(&t))
    }

    /// Whether `t` names a record or a locus: an element held by pointer.
    fn pointer_elem(&self, t: &Ty) -> bool {
        match self.dealias(t) {
            Ty::Prim(PrimType::String | PrimType::Bytes) => true,
            Ty::Named(n) => matches!(
                self.top.lookup(&n),
                Some(TopSymbol::Locus(_)) | Some(TopSymbol::Type(crate::symbol::TypeInfo { kind: TypeKind::Struct(_), .. }))
            ),
            _ => false,
        }
    }

    /// Whether `t` is a `bounded` of pointer-shaped elements.
    fn pointer_bounded(&self, t: &Ty) -> bool {
        matches!(self.dealias(t), Ty::Bounded(e, _) if self.pointer_elem(&e))
    }

    /// Whether a value of type `t` holds such a bounded place, at any depth
    /// of record fields: what a writer may write through.
    fn holds_pointer_bounded(&self, t: &Ty, depth: u32) -> bool {
        if depth > 8 {
            return false;
        }
        let t = self.dealias(t);
        if self.pointer_bounded(&t) {
            return true;
        }
        match &t {
            Ty::Named(n) => match self.top.lookup(n) {
                Some(TopSymbol::Type(info)) => match &info.kind {
                    TypeKind::Struct(fields) => fields.iter().any(|f| self.holds_pointer_bounded(&f.ty, depth + 1)),
                    _ => false,
                },
                _ => false,
            },
            Ty::Array(e, _) => self.holds_pointer_bounded(e, depth + 1),
            _ => false,
        }
    }

    fn name_of(&self, segs: &[String]) -> String {
        let joined = segs.join("::");
        self.renames.get(&joined).cloned().unwrap_or_else(|| segs.last().cloned().unwrap_or_default())
    }

    /// What a call reaches: `Some(name)` for a free fn by name or through a
    /// local bound to one, `None` for a call through a value the walk
    /// cannot name; the intrinsics and anything that is not a call of the
    /// program's free fns are `Some` of their own name.
    fn callee(&self, e: &Expr, b: &Bindings, locals: &BTreeSet<String>) -> Option<String> {
        match e {
            Expr::Ident(i) => {
                if let Some(f) = b.fn_values.get(&i.name) {
                    return Some(f.clone());
                }
                if locals.contains(&i.name) {
                    return None;
                }
                Some(i.name.clone())
            }
            Expr::Path(q) => Some(self.name_of(&q.segments.iter().map(|s| s.name.clone()).collect::<Vec<_>>())),
            // a method: a member's own writes are refused at its intrinsic
            Expr::Field { .. } | Expr::Path2 { .. } => Some(String::new()),
            _ => None,
        }
    }

    /// Record a `let` into `b`: a bounded view (by the place's type), or a
    /// local bound to a free fn's name. `root_ty` types a place's root.
    fn bind(&self, b: &mut Bindings, name: &str, value: &Expr, locals: &BTreeSet<String>, root_ty: &dyn Fn(&str) -> Option<Ty>) {
        b.views.remove(name);
        b.fn_values.remove(name);
        if let Expr::Ident(i) = value {
            if let Some(f) = b.fn_values.get(&i.name).cloned() {
                b.fn_values.insert(name.to_string(), f);
                return;
            }
            if !locals.contains(&i.name) && self.fns.contains_key(&i.name) {
                b.fn_values.insert(name.to_string(), i.name.clone());
                return;
            }
        }
        let Some((root, path)) = place(value) else { return };
        let (root, path) = match b.views.get(&root).cloned() {
            Some((r, p0)) => (r, p0.into_iter().chain(path).collect::<Vec<_>>()),
            None => (root, path),
        };
        let bounded = root_ty(&root).and_then(|t| self.type_at(&t, &path)).is_some_and(|t| matches!(t, Ty::Bounded(..)));
        if bounded {
            b.views.insert(name.to_string(), (root, path));
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
        let types: BTreeMap<String, Ty> = f.params.iter().cloned().collect();
        let root_ty = |r: &str| types.get(r).cloned();
        let mut b = Bindings::default();
        let mut locals: BTreeSet<String> = f.params.iter().map(|(n, _)| n.clone()).collect();
        let mut out = BTreeSet::new();
        visit_block(f.body, &mut |v| match v {
            Visited::Stmt(Stmt::Let { name, value, .. }) => {
                self.bind(&mut b, &name.name, value, &locals, &root_ty);
                locals.insert(name.name.clone());
            }
            Visited::Expr(Expr::Call { callee, args, .. }) => {
                let root_of = |e: &Expr| -> Option<(usize, Vec<Step>)> {
                    let (root, path) = place(e)?;
                    let (root, path) = match b.views.get(&root) {
                        Some((r, p0)) => (r.clone(), p0.iter().cloned().chain(path).collect()),
                        None => (root, path),
                    };
                    Some((*index.get(root.as_str())?, path))
                };
                match self.callee(callee, &b, &locals) {
                    Some(c) if (c == "push" && args.len() == 2) || (c == "set" && args.len() == 3) => {
                        if let Some((i, path)) = root_of(&args[0]) {
                            if self.type_at(&f.params[i].1, &path).is_some_and(|t| self.pointer_bounded(&t)) {
                                out.insert(i);
                            }
                        }
                    }
                    Some(c) => {
                        if let Some(ks) = known.get(&c) {
                            for &k in ks {
                                if let Some((i, _)) = args.get(k).and_then(root_of) {
                                    out.insert(i);
                                }
                            }
                        }
                    }
                    // a fn the walk cannot name may be any writer: every
                    // argument holding such a place is written through
                    None => {
                        for a in args {
                            if let Some((i, path)) = root_of(a) {
                                if self.type_at(&f.params[i].1, &path).is_some_and(|t| self.holds_pointer_bounded(&t, 0)) {
                                    out.insert(i);
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        });
        out
    }

    /// The calls in `body` that hand a writer a place the frame does not
    /// own. `params` are the body's parameters; `locus` is set for a locus
    /// member, whose own parameters' places are not the frame's to write
    /// through either (its arena is its scratch).
    fn calls(
        &self,
        body: &Block,
        params: &BTreeMap<String, Ty>,
        locus: Option<&str>,
        writers: &BTreeMap<String, BTreeSet<usize>>,
        out: &mut Vec<Diag>,
    ) {
        let member = locus.is_some();
        let self_ty = locus.map(|l| Ty::Named(l.to_string()));
        let root_ty = |r: &str| -> Option<Ty> {
            if r == "self" {
                return self_ty.clone();
            }
            params.get(r).cloned()
        };
        let mut b = Bindings::default();
        let mut locals: BTreeSet<String> = params.keys().cloned().collect();
        let mut found: Vec<Diag> = Vec::new();
        visit_block(body, &mut |v| match v {
            Visited::Stmt(Stmt::Let { name, value, .. }) => {
                self.bind(&mut b, &name.name, value, &locals, &root_ty);
                locals.insert(name.name.clone());
            }
            Visited::Expr(Expr::Call { callee, args, .. }) => {
                let reached = self.callee(callee, &b, &locals);
                let ks: Vec<usize> = match &reached {
                    Some(c) => match writers.get(c) {
                        Some(ks) => ks.iter().copied().collect(),
                        None => return,
                    },
                    None => (0..args.len()).collect(),
                };
                for k in ks {
                    let Some(arg) = args.get(k) else { continue };
                    let Some((root, path)) = place(arg) else { continue };
                    let (base, full) = match b.views.get(&root) {
                        Some((r, p0)) => (r.clone(), p0.iter().cloned().chain(path).collect::<Vec<_>>()),
                        None => (root, path),
                    };
                    let foreign = base == "self" || (member && params.contains_key(&base));
                    if !foreign {
                        continue;
                    }
                    if reached.is_none()
                        && !root_ty(&base).and_then(|t| self.type_at(&t, &full)).is_some_and(|t| self.holds_pointer_bounded(&t, 0))
                    {
                        continue;
                    }
                    let shown = match callee.as_ref() {
                        Expr::Path(q) => q.segments.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join("::"),
                        Expr::Ident(i) => i.name.clone(),
                        _ => "this call".to_string(),
                    };
                    let what = if reached.is_some() {
                        format!("`{shown}` stores pointer-shaped elements (a String, Bytes or a record) into a bounded place under its parameter")
                    } else {
                        format!("`{shown}` is a call through a function value whose fn this check cannot name, and may store pointer-shaped elements (a String, Bytes or a record) into a bounded place under its parameter")
                    };
                    let whose = if base == "self" { "this locus's arena" } else { "its caller's frame, not this member's" };
                    found.push(Diag::ty(
                        arg.span(),
                        format!(
                            "{what}, keeping them in the frame that calls it; this argument is a place in {whose}, so \
                             it would hold elements that frame frees. Pass a local and store it whole into the place \
                             after the call, or write the elements through `self` in a method of the locus that \
                             holds the place"
                        ),
                    ));
                }
            }
            _ => {}
        });
        out.extend(found);
    }
}
