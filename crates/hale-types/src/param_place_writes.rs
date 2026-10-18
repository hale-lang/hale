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
//! check: nothing at run time. Names are bindings, by the snapshot's
//! identity of each use (`Snapshot::binding_of`): `let r = r;` is a binding
//! of its own, not the parameter, and an inner `let f` names nothing after
//! its block. A bounded local is a view only when its type is a `bounded`
//! (a `let` of a record copies it), and a call through a function value
//! names the fn the value is bound to (`let f = fill;`, a tuple `let`'s
//! element of a tuple literal); one whose fn the walk cannot name (a
//! parameter of fn type, a computed callee, a binding some statement
//! assigns, `f = other;`) is taken as a writer of every argument whose
//! type holds such a place: refused rather than guessed. A generic fn is
//! summarized as any other, a type parameter taken as any type (a
//! pointer-shaped element, a record holding such a place, a field path
//! through it the template cannot resolve).

use std::collections::{BTreeMap, BTreeSet};

use hale_syntax::ast::{flat_decls, Block, Expr, Ident, LocusMember, NodeId, Param, PrimType, Program, Stmt, TopDecl, TypeExpr};
use hale_syntax::error::Diag;

use crate::listener_window::{visit_block, Visited};
use crate::resolve::TopScope;
use crate::snapshot::Snapshot;
use crate::symbol::{Bundle, TopSymbol, TypeKind};
use crate::ty::Ty;

/// Every call in the program's bodies that hands a free fn writing
/// through a parameter a place the frame does not own.
pub(crate) fn param_place_write_diags(bundle: &Bundle<'_>, top: &TopScope) -> Vec<Diag> {
    let mut programs: Vec<(&Program, Ids<'_>)> =
        bundle.programs.values().map(|p| (*p, Ids(Some(&bundle.snapshot)))).collect();
    let user = programs.len();
    if let Some(std_prog) = crate::stdlib_bodies::program() {
        programs.push((std_prog, Ids(crate::stdlib_bodies::identities())));
    }
    let world = World::gather(&programs, &bundle.import_renames, top);
    let writers = world.writers();
    let mut out = Vec::new();
    for (p, ids) in &programs[..user] {
        for d in flat_decls(&p.items) {
            let params = |ps: &[Param]| -> BTreeMap<Key, Ty> {
                ps.iter().map(|p| (ids.decl(&p.name), world.ty(&p.ty))).collect()
            };
            match d {
                TopDecl::Fn(f) => world.calls(&f.body, ids, &params(&f.params), None, &writers, &mut out),
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        let (body, ps) = match m {
                            LocusMember::Fn(f) => (&f.body, params(&f.params)),
                            LocusMember::Lifecycle(lc) => (&lc.body, params(&lc.params)),
                            LocusMember::Mode(md) => (&md.body, params(&md.params)),
                            LocusMember::Failure(fd) => (&fd.body, params(&fd.params)),
                            _ => continue,
                        };
                        world.calls(body, ids, &ps, Some(&l.name.name), &writers, &mut out);
                    }
                }
                _ => {}
            }
        }
    }
    out
}

/// A body's binding, by identity: the declaration a snapshot minted (a
/// parameter, a `let`, a tuple `let`'s name, a `for` or pattern binder),
/// so an inner `let f` or a `let r = r;` is its own binding and a use
/// after its block names the outer one again; `self`; or, in a program no
/// snapshot minted, the spelling.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Key {
    SelfKw,
    Decl(u32),
    Name(String),
}

/// The identities of one program's bodies.
#[derive(Clone, Copy)]
struct Ids<'s>(Option<&'s Snapshot>);

impl Ids<'_> {
    /// The binding a declaration's name (a parameter, a tuple `let`'s
    /// name: a binder, whose id is its `Ident`'s) introduces.
    fn decl(&self, name: &Ident) -> Key {
        self.decl_at(&name.name, name.id)
    }

    /// The binding a declaration with the site `id` introduces.
    fn decl_at(&self, name: &str, id: NodeId) -> Key {
        if id.is_none() || self.0.is_none() {
            Key::Name(name.to_string())
        } else {
            Key::Decl(id.0)
        }
    }

    /// The binding the use `at` of `name` (an identifier expression, or an
    /// assignment's head by its statement's id) names; a minted use naming
    /// no local is its spelling, which no local's key is.
    fn use_at(&self, name: &str, at: NodeId) -> Key {
        match self.0.and_then(|s| s.declaration_of(at)) {
            Some(d) => Key::Decl(d.index),
            None => Key::Name(name.to_string()),
        }
    }
}

struct FreeFn<'a, 's> {
    params: Vec<(Key, Ty)>,
    body: &'a Block,
    ids: Ids<'s>,
}

struct World<'a, 't, 's> {
    top: &'t TopScope,
    fns: BTreeMap<String, FreeFn<'a, 's>>,
    renames: BTreeMap<String, String>,
}

#[derive(Clone)]
enum Step {
    Field(String),
    Index,
}

/// How a body reaches the bounded places of its roots, and which of its
/// bindings it can name a fn for.
struct Bindings<'s> {
    ids: Ids<'s>,
    /// Every binding the body declares (`Key::Name` ones; a minted use
    /// that names a declaration is a local whatever it is).
    locals: BTreeSet<Key>,
    /// A bounded local bound from a place: its root and the place's path.
    views: BTreeMap<Key, (Key, Vec<Step>)>,
    /// A local bound to a free fn's name.
    fn_values: BTreeMap<Key, String>,
    /// The bindings some statement of the body assigns whole
    /// (`f = other;`): which fn such a binding holds at a call depends on
    /// the path taken, so the walk names none for it.
    assigned: BTreeSet<Key>,
}

impl<'s> Bindings<'s> {
    fn new(ids: Ids<'s>, body: &Block, params: impl IntoIterator<Item = Key>) -> Bindings<'s> {
        let mut assigned = BTreeSet::new();
        visit_block(body, &mut |v| {
            if let Visited::Stmt(Stmt::Assign { target, id, .. }) = v {
                if target.tail.is_empty() {
                    assigned.insert(ids.use_at(&target.head.name, *id));
                }
            }
        });
        Bindings { ids, locals: params.into_iter().collect(), views: BTreeMap::new(), fn_values: BTreeMap::new(), assigned }
    }

    fn is_local(&self, k: &Key) -> bool {
        matches!(k, Key::Decl(_)) || self.locals.contains(k)
    }

    /// The root of a place expression and its field path: `self.a.b` is
    /// (`self`, [a, b]); `x[i].y` is (`x`, [y]), an index stepping into an
    /// element (`Index` in the path); a bounded local viewing a place is
    /// that place's root, its path before the rest.
    fn place(&self, e: &Expr) -> Option<(Key, Vec<Step>)> {
        let (root, path) = self.place_written(e)?;
        Some(match self.views.get(&root) {
            Some((r, p0)) => (r.clone(), p0.iter().cloned().chain(path).collect()),
            None => (root, path),
        })
    }

    fn place_written(&self, e: &Expr) -> Option<(Key, Vec<Step>)> {
        match e {
            Expr::Ident(i) => Some((self.ids.use_at(&i.name, i.id), Vec::new())),
            Expr::KwSelf(_) => Some((Key::SelfKw, Vec::new())),
            Expr::Field { receiver, name, .. } => {
                let (root, mut path) = self.place_written(receiver)?;
                path.push(Step::Field(name.name.clone()));
                Some((root, path))
            }
            Expr::Index { receiver, .. } => {
                let (root, mut path) = self.place_written(receiver)?;
                path.push(Step::Index);
                Some((root, path))
            }
            _ => None,
        }
    }
}

impl<'a, 't, 's> World<'a, 't, 's> {
    fn gather(programs: &[(&'a Program, Ids<'s>)], import_renames: &[(Vec<String>, String)], top: &'t TopScope) -> World<'a, 't, 's> {
        let mut w = World { top, fns: BTreeMap::new(), renames: BTreeMap::new() };
        for (path, name) in hale_stdlib::PATH_RENAMES {
            w.renames.insert(path.join("::"), name.to_string());
        }
        for (path, mangled) in import_renames {
            w.renames.insert(path.join("::"), mangled.clone());
        }
        for (p, ids) in programs {
            for d in flat_decls(&p.items) {
                if let TopDecl::Fn(f) = d {
                    if f.ffi.is_none() {
                        let params = f.params.iter().map(|p| (ids.decl(&p.name), w.ty(&p.ty))).collect();
                        w.fns.insert(f.name.name.clone(), FreeFn { params, body: &f.body, ids: *ids });
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

    /// Whether the path below a value of type `t` passes through a type the
    /// walk cannot see into: a generic fn's type parameter (`fn fill<T>(r:
    /// T) { push(r.xs, s) }` reads `r.xs` of a `T`), whose instance may be a
    /// record holding a bounded place there.
    fn opaque_on(&self, t: &Ty, path: &[Step]) -> bool {
        let mut t = self.dealias(t);
        for s in path {
            if self.unseen(&t) {
                return true;
            }
            match self.step(&t, s) {
                Some(next) => t = self.dealias(&next),
                None => return false,
            }
        }
        self.unseen(&t)
    }

    /// A type no declaration of the program names: a type parameter.
    fn unseen(&self, t: &Ty) -> bool {
        match t {
            Ty::Named(n) => self.top.lookup(n).is_none(),
            Ty::Unknown => true,
            _ => false,
        }
    }

    /// Whether a place of type `t` at `path` may be a bounded place of
    /// pointer-shaped elements (`holds`: or hold one, at any depth): known
    /// by its type, or unknown through a type parameter (fail closed).
    fn may_write(&self, t: &Ty, path: &[Step], holds: bool) -> bool {
        match self.type_at(t, path) {
            Some(at) if holds => self.holds_pointer_bounded(&at, 0),
            Some(at) => self.pointer_bounded(&at),
            None => self.opaque_on(t, path),
        }
    }

    /// Whether `t` names a record or a locus: an element held by pointer.
    fn pointer_elem(&self, t: &Ty) -> bool {
        match self.dealias(t) {
            Ty::Prim(PrimType::String | PrimType::Bytes) => true,
            Ty::Named(n) => matches!(
                self.top.lookup(&n),
                Some(TopSymbol::Locus(_)) | Some(TopSymbol::Type(crate::symbol::TypeInfo { kind: TypeKind::Struct(_), .. })) | None
            ),
            // a generic fn's type parameter may be any of them: fail closed
            Ty::Unknown => true,
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
                // a generic fn's type parameter may be such a record
                None => true,
                _ => false,
            },
            Ty::Unknown => true,
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
    fn callee(&self, e: &Expr, b: &Bindings<'_>) -> Option<String> {
        match e {
            Expr::Ident(i) => {
                let k = b.ids.use_at(&i.name, i.id);
                if let Some(f) = b.fn_values.get(&k) {
                    return Some(f.clone());
                }
                if b.is_local(&k) {
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

    /// Record a binding of `name` (key `k`) to `value` into `b`: a bounded
    /// view (by the place's type), or a local bound to a free fn's name
    /// (unless the body assigns it). `root_ty` types a place's root.
    fn bind(&self, b: &mut Bindings<'_>, k: Key, value: &Expr, root_ty: &dyn Fn(&Key) -> Option<Ty>) {
        b.views.remove(&k);
        b.fn_values.remove(&k);
        b.locals.insert(k.clone());
        if let Expr::Ident(i) = value {
            let from = b.ids.use_at(&i.name, i.id);
            let named = match b.fn_values.get(&from) {
                Some(f) => Some(f.clone()),
                None if !b.is_local(&from) && self.fns.contains_key(&i.name) => Some(i.name.clone()),
                None => None,
            };
            if let Some(f) = named {
                if !b.assigned.contains(&k) {
                    b.fn_values.insert(k, f);
                }
                return;
            }
        }
        let Some((root, path)) = b.place(value) else { return };
        let bounded = root_ty(&root).and_then(|t| self.type_at(&t, &path)).is_some_and(|t| matches!(t, Ty::Bounded(..)));
        if bounded {
            b.views.insert(k, (root, path));
        }
    }

    /// The bindings a statement introduces: a `let`'s name, and a tuple
    /// `let`'s names, each bound to its element of a tuple literal (any
    /// other value binds them to nothing the walk names).
    fn bind_stmt(&self, b: &mut Bindings<'_>, s: &Stmt, root_ty: &dyn Fn(&Key) -> Option<Ty>) {
        match s {
            Stmt::Let { name, value, id, .. } => {
                let k = b.ids.decl_at(&name.name, *id);
                self.bind(b, k, value, root_ty);
            }
            Stmt::LetTuple { names, value, .. } => {
                let parts = match value {
                    Expr::Tuple(es, _) if es.len() == names.len() => Some(es),
                    _ => None,
                };
                for (j, n) in names.iter().enumerate() {
                    let k = b.ids.decl(n);
                    match parts {
                        Some(es) => self.bind(b, k, &es[j], root_ty),
                        None => {
                            b.views.remove(&k);
                            b.fn_values.remove(&k);
                            b.locals.insert(k);
                        }
                    }
                }
            }
            _ => {}
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
    fn writes_of(&self, f: &FreeFn<'a, 's>, known: &BTreeMap<String, BTreeSet<usize>>) -> BTreeSet<usize> {
        let index: BTreeMap<&Key, usize> = f.params.iter().enumerate().map(|(i, (k, _))| (k, i)).collect();
        let types: BTreeMap<Key, Ty> = f.params.iter().cloned().collect();
        let root_ty = |r: &Key| types.get(r).cloned();
        let mut b = Bindings::new(f.ids, f.body, f.params.iter().map(|(k, _)| k.clone()));
        let mut out = BTreeSet::new();
        visit_block(f.body, &mut |v| match v {
            Visited::Stmt(s @ (Stmt::Let { .. } | Stmt::LetTuple { .. })) => self.bind_stmt(&mut b, s, &root_ty),
            Visited::Expr(Expr::Call { callee, args, .. }) => {
                let root_of = |e: &Expr| -> Option<(usize, Vec<Step>)> {
                    let (root, path) = b.place(e)?;
                    Some((*index.get(&root)?, path))
                };
                match self.callee(callee, &b) {
                    Some(c) if (c == "push" && args.len() == 2) || (c == "set" && args.len() == 3) => {
                        if let Some((i, path)) = root_of(&args[0]) {
                            if self.may_write(&f.params[i].1, &path, false) {
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
                                if self.may_write(&f.params[i].1, &path, true) {
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
        ids: &Ids<'_>,
        params: &BTreeMap<Key, Ty>,
        locus: Option<&str>,
        writers: &BTreeMap<String, BTreeSet<usize>>,
        out: &mut Vec<Diag>,
    ) {
        let member = locus.is_some();
        let self_ty = locus.map(|l| Ty::Named(l.to_string()));
        let root_ty = |r: &Key| -> Option<Ty> {
            if *r == Key::SelfKw {
                return self_ty.clone();
            }
            params.get(r).cloned()
        };
        let mut b = Bindings::new(*ids, body, params.keys().cloned());
        let mut found: Vec<Diag> = Vec::new();
        visit_block(body, &mut |v| match v {
            Visited::Stmt(s @ (Stmt::Let { .. } | Stmt::LetTuple { .. })) => self.bind_stmt(&mut b, s, &root_ty),
            Visited::Expr(Expr::Call { callee, args, .. }) => {
                let reached = self.callee(callee, &b);
                let ks: Vec<usize> = match &reached {
                    Some(c) => match writers.get(c) {
                        Some(ks) => ks.iter().copied().collect(),
                        None => return,
                    },
                    None => (0..args.len()).collect(),
                };
                for k in ks {
                    let Some(arg) = args.get(k) else { continue };
                    let Some((base, full)) = b.place(arg) else { continue };
                    let foreign = base == Key::SelfKw || (member && params.contains_key(&base));
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
                    let whose = if base == Key::SelfKw { "this locus's arena" } else { "its caller's frame, not this member's" };
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
