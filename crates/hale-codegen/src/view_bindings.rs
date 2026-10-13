//! Bindings that view a field another write can replace under them.
//!
//! A binding of a String or Bytes read from a place (`let s =
//! self.f.s;`) holds the field's own pointer: lowering copies a record
//! read from a place (GH #713) but not a String, so the hot path of
//! reading a field into a local costs nothing. The pointer is only good
//! while the field keeps that value. A write to the field replaces it:
//! a same-length String or Bytes assign overwrites the bytes in place
//! (spec/memory.md rule 7), and a replace that does not fit, a whole
//! record store, or a payload enum's, retires the old block, reusable
//! after the next activation-boundary flush (rule 6). A callee that
//! replaces the field runs its flush when it returns, so a local held
//! across the call read whatever reused the block next — `let s =
//! self.f.s; self.put("NEW"); s` printed a later value, and under ASan
//! read freed memory. A match arm's payload binding of a field
//! (`match self.e { E::A(s) => .. }`), a `for` element of an array
//! field, a tuple let's elements and a bare local reassigned from a
//! field are the same view.
//!
//! This pass finds the bindings for which that can happen, so lowering
//! binds an owned copy of exactly those and leaves every other one a
//! view. A value views the places it may have been read from, named by
//! their field names, its keys: a field read (`self.a.b`, `h.name`,
//! `self.arr[i].s`) views its field and what it is read from, and a
//! value that carries another out — a `match` arm's payload, an `if`
//! or block result, a tuple, array or record literal holding it, a
//! call that may hand its argument back — views what that one does. A
//! binding needs its own copy when some use of it comes after (or as
//! an argument of) a write that may change one of its keys:
//!
//! - an assignment `x.f... = v` (any root) writes key `f`;
//! - a call writes what its callee writes, transitively: a locus
//!   method by name, over every locus declaring one (the receiver's
//!   type is not known here), a free fn by name, and a method called
//!   on a field (`self.v.set(..)`, a form) writes that field too;
//! - a call through a fn value (a local, a parameter, a fn-typed
//!   field), a publish, an explicit `yield`, a recovery statement and
//!   a stdlib leaf that publishes or parks on an `async_io` pool (where
//!   another delivery to the same locus runs meanwhile) may write
//!   anything.
//!
//! In a loop, a write anywhere in the body reaches a use anywhere in it
//! when the variable was declared before the loop, wherever in the
//! loop the instance holding the value was assigned. Field names stand in for
//! fields of any type, so the answer only errs toward a copy.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use hale_syntax::ast::*;

/// The binding sites lowering binds an owned copy at. A `match` is
/// named by its span (a `MatchStmt` has no id); every other site by its
/// snapshot id.
#[derive(Debug, Clone, Default)]
pub(crate) struct OwnedViews {
    sites: HashSet<Site>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Site {
    /// A `let`, a tuple `let`, a bare-local `=` or a `for`, by id.
    Node(u32),
    /// A `match`'s payload bindings, by the match's span.
    Match(u32, u32),
    /// A call's argument read from a place the callee may replace (the
    /// callee reads its parameter after), by the argument's span.
    Arg(u32, u32),
    /// A method's returned value that views a place, by the returned
    /// expression's span: handed back as an owned copy, where the
    /// return's same-arena skip would pass the field's own pointer
    /// through to a caller whose arena holds it.
    Ret(u32, u32),
}

impl OwnedViews {
    /// Run the pass over every body of `program`.
    pub(crate) fn of(program: &Program) -> Self {
        let mut out = OwnedViews::default();
        for (site, _) in owned_view_sites(program) {
            out.sites.insert(site);
        }
        out
    }

    /// Does the `let`, tuple `let`, bare-local `=` or `for` with this id
    /// bind an owned copy?
    pub(crate) fn node(&self, id: NodeId) -> bool {
        !id.is_none() && self.sites.contains(&Site::Node(id.0))
    }

    /// Do this match's payload bindings bind owned copies?
    pub(crate) fn match_arms(&self, m: &MatchStmt) -> bool {
        self.sites.contains(&Site::Match(m.span.start.0, m.span.end.0))
    }

    /// Does the returned expression `e` view a place, so the return
    /// hands back an owned copy?
    pub(crate) fn ret(&self, e: &Expr) -> bool {
        let span = e.span();
        !self.sites.is_empty() && self.sites.contains(&Site::Ret(span.start.0, span.end.0))
    }

    /// Is `e` a call's argument read from a place the callee may
    /// replace, passed as an owned copy?
    pub(crate) fn arg(&self, e: &Expr) -> bool {
        if self.sites.is_empty() {
            return false;
        }
        let span = e.span();
        self.sites.contains(&Site::Arg(span.start.0, span.end.0))
    }
}

/// What a body may write: every field name, or these.
#[derive(Debug, Clone, Default, PartialEq)]
struct Writes {
    top: bool,
    fields: BTreeSet<String>,
}

impl Writes {
    fn top() -> Self {
        Writes { top: true, fields: BTreeSet::new() }
    }
    fn field(name: &str) -> Self {
        let mut w = Writes::default();
        w.fields.insert(name.to_string());
        w
    }
    fn add(&mut self, o: &Writes) {
        self.top |= o.top;
        if !self.top {
            self.fields.extend(o.fields.iter().cloned());
        }
    }
    fn is_empty(&self) -> bool {
        !self.top && self.fields.is_empty()
    }
    fn touches(&self, keys: &BTreeSet<String>) -> bool {
        self.top || keys.iter().any(|k| self.fields.contains(k))
    }
}

/// The field names a value may view, or `None` for a value that
/// views no place.
type Keys = Option<BTreeSet<String>>;

fn union(a: Keys, b: Keys) -> Keys {
    match (a, b) {
        (Some(mut a), Some(b)) => {
            a.extend(b);
            Some(a)
        }
        (a, None) => a,
        (None, b) => b,
    }
}

/// What a call names: resolved against the program's summaries once
/// they are complete.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Callee {
    Method(String),
    Fn(String),
}

/// Every body of the program, with the summaries the pass reads.
struct Bodies<'a> {
    bodies: Vec<(&'a [Param], &'a Block)>,
    /// Method name → the summaries' index of each body declaring it.
    methods: BTreeMap<String, Vec<usize>>,
    fns: BTreeMap<String, Vec<usize>>,
    /// Field names declared with a fn type: `x.cb(..)` calls a value.
    fn_fields: BTreeSet<String>,
}

fn collect<'a>(items: &'a [TopDecl], p: &mut Bodies<'a>) {
    for item in items {
        match item {
            TopDecl::Fn(f) => {
                p.fns.entry(f.name.name.clone()).or_default().push(p.bodies.len());
                p.bodies.push((&f.params, &f.body));
            }
            TopDecl::Locus(l) => {
                for m in &l.members {
                    match m {
                        LocusMember::Fn(f) => {
                            p.methods.entry(f.name.name.clone()).or_default().push(p.bodies.len());
                            p.bodies.push((&f.params, &f.body));
                        }
                        LocusMember::Lifecycle(d) => p.bodies.push((&d.params, &d.body)),
                        LocusMember::Mode(d) => p.bodies.push((&d.params, &d.body)),
                        LocusMember::Failure(d) => p.bodies.push((&d.params, &d.body)),
                        LocusMember::Params(pb) => {
                            for d in &pb.params {
                                if matches!(d.ty, Some(TypeExpr::Function { .. })) {
                                    p.fn_fields.insert(d.name.name.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            TopDecl::Perspective(d) => {
                for m in &d.members {
                    if let PerspectiveMember::Fn(f) = m {
                        p.methods.entry(f.name.name.clone()).or_default().push(p.bodies.len());
                        p.bodies.push((&f.params, &f.body));
                    }
                }
            }
            TopDecl::Module(m) => collect(&m.items, p),
            _ => {}
        }
    }
}

/// The flagged sites, each with the name it binds (the first name of a
/// tuple `let`, a match's first payload binding): for the tests.
fn owned_view_sites(program: &Program) -> Vec<(Site, String)> {
    let mut p = Bodies {
        bodies: Vec::new(),
        methods: BTreeMap::new(),
        fns: BTreeMap::new(),
        fn_fields: BTreeSet::new(),
    };
    collect(&program.items, &mut p);

    // Each body's own writes and the calls it makes, then the fixpoint.
    let mut direct: Vec<(Writes, BTreeSet<Callee>)> = Vec::with_capacity(p.bodies.len());
    for (params, body) in &p.bodies {
        let mut w = Walk::new(&p, None, params);
        w.block(body);
        direct.push((w.own, w.callees));
    }
    let mut summary: Vec<Writes> = direct.iter().map(|(w, _)| w.clone()).collect();
    loop {
        let mut changed = false;
        for i in 0..summary.len() {
            let mut next = summary[i].clone();
            for c in &direct[i].1 {
                for j in p.resolve(c) {
                    let sj = summary[j].clone();
                    next.add(&sj);
                }
            }
            if next != summary[i] {
                summary[i] = next;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let mut out = Vec::new();
    for (params, body) in &p.bodies {
        let mut w = Walk::new(&p, Some(&summary), params);
        // A body's tail is its value when the fn returns one.
        let keys = w.block(body);
        if let Some(t) = &body.tail {
            w.returned(t, keys);
        }
        out.extend(w.flagged());
    }
    out
}

impl Bodies<'_> {
    fn resolve(&self, c: &Callee) -> Vec<usize> {
        let m = match c {
            Callee::Method(n) => self.methods.get(n),
            Callee::Fn(n) => self.fns.get(n),
        };
        m.cloned().unwrap_or_default()
    }
}

/// One binding of a variable: its site, the place keys it views, and
/// where it was made.
struct Inst {
    var: usize,
    site: Site,
    name: String,
    keys: BTreeSet<String>,
    at: u32,
}

/// A write event: the writes, and the positions it spans (a call's
/// arguments are inside it: a callee reads its parameters after it
/// writes).
struct WriteAt {
    writes: Writes,
    start: u32,
    at: u32,
}

struct Walk<'p, 'a> {
    p: &'p Bodies<'a>,
    /// `None` on the summary pass: collect `own` and `callees` only.
    summary: Option<&'p [Writes]>,
    own: Writes,
    callees: BTreeSet<Callee>,
    pos: u32,
    scopes: Vec<BTreeMap<String, Option<usize>>>,
    /// Per variable, its instances.
    vars: Vec<Vec<usize>>,
    /// Per variable, where it was declared: a loop that starts after
    /// it carries its value round the back edge, whichever instance of
    /// it holds the value.
    declared: Vec<u32>,
    insts: Vec<Inst>,
    /// Per variable, the positions it is read at.
    uses: Vec<Vec<u32>>,
    writes: Vec<WriteAt>,
    loops: Vec<(u32, u32)>,
    /// Arguments passed to a call that may replace what they view, and
    /// returned values that view a place.
    args: Vec<(Site, String)>,
}

fn pattern_names<'e>(p: &'e Pattern, out: &mut Vec<&'e Ident>) {
    match p {
        Pattern::Binding(id) => out.push(id),
        Pattern::Constructor { args, .. } => args.iter().for_each(|a| pattern_names(a, out)),
        Pattern::Tuple(ps, _) => ps.iter().for_each(|a| pattern_names(a, out)),
        Pattern::Literal(..) | Pattern::Wildcard(_) => {}
    }
}

impl<'p, 'a> Walk<'p, 'a> {
    fn new(p: &'p Bodies<'a>, summary: Option<&'p [Writes]>, params: &[Param]) -> Self {
        let mut top = BTreeMap::new();
        for prm in params {
            top.insert(prm.name.name.clone(), None);
        }
        Walk {
            p,
            summary,
            own: Writes::default(),
            callees: BTreeSet::new(),
            pos: 0,
            scopes: vec![top],
            vars: Vec::new(),
            declared: Vec::new(),
            insts: Vec::new(),
            uses: Vec::new(),
            writes: Vec::new(),
            loops: Vec::new(),
            args: Vec::new(),
        }
    }

    fn tick(&mut self) -> u32 {
        self.pos += 1;
        self.pos
    }

    /// `Some(Some(var))` a local, `Some(None)` a parameter (or a
    /// binding this pass does not track), `None` not a local at all.
    fn lookup(&self, name: &str) -> Option<Option<usize>> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn declare(&mut self, name: &str) -> usize {
        let v = self.vars.len();
        self.vars.push(Vec::new());
        self.declared.push(self.pos);
        self.uses.push(Vec::new());
        self.scopes.last_mut().expect("a scope").insert(name.to_string(), Some(v));
        v
    }

    /// The keys every instance of the variable `v` views.
    fn var_keys(&self, v: usize) -> Keys {
        let mut all = BTreeSet::new();
        for &i in &self.vars[v] {
            all.extend(self.insts[i].keys.iter().cloned());
        }
        (!all.is_empty()).then_some(all)
    }

    fn bind(&mut self, var: usize, site: Site, name: &str, keys: Option<BTreeSet<String>>) {
        let at = self.tick();
        let Some(keys) = keys else { return };
        let i = self.insts.len();
        self.insts.push(Inst { var, site, name: name.to_string(), keys, at });
        self.vars[var].push(i);
    }

    fn write(&mut self, writes: Writes, start: Option<u32>) {
        let at = self.tick();
        self.own.add(&writes);
        if !writes.is_empty() {
            self.writes.push(WriteAt { writes, start: start.unwrap_or(at), at });
        }
    }

    /// Walk a block; the keys of its value (its tail's).
    fn block(&mut self, b: &Block) -> Keys {
        self.scopes.push(BTreeMap::new());
        for s in &b.stmts {
            self.stmt(s);
        }
        let keys = b.tail.as_ref().and_then(|t| self.expr(t));
        self.scopes.pop();
        keys
    }

    fn in_loop(&mut self, f: impl FnOnce(&mut Self)) {
        let start = self.tick();
        f(self);
        let end = self.tick();
        self.loops.push((start, end));
    }

    /// Walk a statement; the keys of its value when it is an
    /// expression, `if`, `match` or block statement.
    fn stmt(&mut self, s: &Stmt) -> Keys {
        match s {
            Stmt::Let { name, value, id, .. } => {
                let keys = self.expr(value);
                let v = self.declare(&name.name);
                self.bind(v, Site::Node(id.0), &name.name, keys);
            }
            Stmt::LetTuple { names, value, id, .. } => {
                let keys = self.expr(value);
                for n in names {
                    let v = self.declare(&n.name);
                    self.bind(v, Site::Node(id.0), &n.name, keys.clone());
                }
            }
            Stmt::Assign { target, value, id, .. } => {
                for seg in &target.tail {
                    if let LValueSeg::Index(e) = seg {
                        self.expr(e);
                    }
                }
                let keys = self.expr(value);
                let first = target.tail.iter().find_map(|seg| match seg {
                    LValueSeg::Field(f) => Some(f.name.as_str()),
                    LValueSeg::Index(_) => None,
                });
                if target.tail.is_empty() {
                    // A bare local rebound: a new instance of it.
                    if let Some(Some(v)) = self.lookup(&target.head.name) {
                        self.bind(v, Site::Node(id.0), &target.head.name, keys);
                    }
                } else if let Some(f) = first {
                    self.write(Writes::field(f), None);
                } else {
                    // `x[i] = v` on a local or a parameter: the element
                    // a binding of `x[j]` views.
                    self.use_name(&target.head.name);
                    self.write(Writes::field(&target.head.name), None);
                }
            }
            Stmt::If(i) => return self.if_stmt(i),
            Stmt::Match(m) => return self.match_stmt(m),
            Stmt::For { name, iter, body, id, .. } => {
                // The loop holds the array it walks from before its
                // first iteration to its last, reading the next element
                // at the top of each: the binding is the array's, made
                // before the loop, and the element is that binding.
                let keys = self.expr(iter);
                self.scopes.push(BTreeMap::new());
                let v = self.declare(&name.name);
                self.bind(v, Site::Node(id.0), &name.name, keys);
                self.in_loop(|w| {
                    let at = w.tick();
                    w.uses[v].push(at);
                    w.block(body);
                });
                self.scopes.pop();
            }
            Stmt::While { cond, body, .. } => self.in_loop(|w| {
                w.expr(cond);
                w.block(body);
            }),
            Stmt::Return(e, _) => {
                if let Some(e) = e {
                    let keys = self.expr(e);
                    self.returned(e, keys);
                }
            }
            Stmt::Fail { value, .. } => {
                self.expr(value);
            }
            Stmt::Violate { payload, .. } => {
                if let Some(e) = payload {
                    self.expr(e);
                }
            }
            Stmt::Send { subject, value, or_disposition, .. } => {
                self.expr(subject);
                self.expr(value);
                if let Some(d) = or_disposition {
                    self.disposition(d);
                }
                // A direct-dispatch subscriber runs before the publish
                // returns.
                self.write(Writes::top(), None);
            }
            Stmt::Yield(_) | Stmt::Reperspective { .. } => self.write(Writes::top(), None),
            Stmt::Recovery { args, .. } => {
                args.iter().for_each(|a| {
                    self.expr(a);
                });
                self.write(Writes::top(), None);
            }
            Stmt::ShmWrite { max, binding, body, .. } => {
                self.expr(max);
                self.scopes.push(BTreeMap::new());
                self.scopes.last_mut().expect("a scope").insert(binding.name.clone(), None);
                self.block(body);
                self.scopes.pop();
            }
            Stmt::Block(b) => return self.block(b),
            Stmt::Expr(e) => return self.expr(e),
            Stmt::Break(_) | Stmt::Continue(_) | Stmt::Terminate(_) => {}
        }
        None
    }

    /// Walk an `if`; the keys of its value, either branch's.
    fn if_stmt(&mut self, i: &IfStmt) -> Keys {
        self.expr(&i.cond);
        let then = self.block(&i.then_block);
        let other = match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self.block(b),
            Some(ElseBranch::ElseIf(e)) => self.if_stmt(e),
            None => None,
        };
        union(then, other)
    }

    /// Walk a `match`; the keys of its value, any arm's.
    fn match_stmt(&mut self, m: &MatchStmt) -> Keys {
        let keys = self.expr(&m.scrutinee);
        let mut out = None;
        let site = Site::Match(m.span.start.0, m.span.end.0);
        for arm in &m.arms {
            self.scopes.push(BTreeMap::new());
            let mut names = Vec::new();
            pattern_names(&arm.pattern, &mut names);
            for n in names {
                let v = self.declare(&n.name);
                self.bind(v, site, &n.name, keys.clone());
            }
            if let Some(g) = &arm.guard {
                self.expr(g);
            }
            let arm_keys = match &arm.body {
                MatchArmBody::Expr(e) => self.expr(e),
                MatchArmBody::Block(b) => self.block(b),
            };
            out = union(out, arm_keys);
            self.scopes.pop();
        }
        out
    }

    /// Walk an `or`'s disposition; the keys of a substitute value.
    fn disposition(&mut self, d: &OrDisposition) -> Keys {
        match d {
            OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => {
                self.scopes.push(BTreeMap::new());
                self.scopes.last_mut().expect("a scope").insert("err".into(), None);
                let keys = self.expr(e);
                self.scopes.pop();
                if matches!(d, OrDisposition::Substitute(_)) { keys } else { None }
            }
            OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) => None,
        }
    }

    /// A returned value that views a place is handed back owned.
    fn returned(&mut self, e: &Expr, keys: Keys) {
        if let (Some(keys), true) = (keys, self.summary.is_some()) {
            let span = e.span();
            let name = keys.iter().next().cloned().unwrap_or_default();
            self.args.push((Site::Ret(span.start.0, span.end.0), name));
        }
    }

    fn use_name(&mut self, name: &str) {
        if let Some(Some(v)) = self.lookup(name) {
            let at = self.tick();
            self.uses[v].push(at);
        }
    }

    /// Walk an expression; the keys of the places its value may view
    /// (`None`: it views none). A field read views its field, and
    /// whatever the value it is read from views; a local, what its
    /// instances view; a tuple, array or record literal, what its
    /// parts view; a block, `if`, `match` or `or`, what any value it
    /// may produce views. A call's result may hand back what its
    /// arguments and its receiver view (a stdlib leaf may return its
    /// argument, an enum constructor holds it), so it views their
    /// keys; arithmetic and concatenation produce fresh values.
    fn expr(&mut self, e: &Expr) -> Keys {
        match e {
            Expr::Ident(id) => {
                self.use_name(&id.name);
                match self.lookup(&id.name) {
                    Some(Some(v)) => self.var_keys(v),
                    _ => None,
                }
            }
            Expr::Literal(..) | Expr::Path(_) | Expr::KwSelf(_) => None,
            Expr::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
                None
            }
            Expr::Unary { operand, .. } => {
                self.expr(operand);
                None
            }
            Expr::Call { callee, args, .. } => self.call(callee, args),
            Expr::Field { receiver, name, .. } => {
                let mut keys = self.expr(receiver).unwrap_or_default();
                keys.insert(name.name.clone());
                Some(keys)
            }
            Expr::Path2 { receiver, .. } => {
                self.expr(receiver);
                None
            }
            Expr::Index { receiver, index, .. } => {
                let keys = self.expr(receiver);
                self.expr(index);
                keys
            }
            Expr::Tuple(es, _) | Expr::Array(es, _) => {
                es.iter().fold(None, |acc, x| union(acc, self.expr(x)))
            }
            Expr::Struct { inits, .. } => inits.iter().fold(None, |acc, i| union(acc, self.expr(&i.value))),
            Expr::Block(b) => self.block(b),
            Expr::If(i) => self.if_stmt(i),
            Expr::Match(m) => self.match_stmt(m),
            Expr::Sum(x, _) | Expr::Prod(x, _) => {
                self.expr(x);
                None
            }
            Expr::Approx { left, right, tolerance, .. } => {
                self.expr(left);
                self.expr(right);
                self.expr(tolerance);
                None
            }
            Expr::Range { lo, hi, .. } => {
                self.expr(lo);
                self.expr(hi);
                None
            }
            Expr::ArrayRepeat { val, .. } => self.expr(val),
            Expr::Or { inner, disposition, .. } => {
                let keys = self.expr(inner);
                let other = self.disposition(disposition);
                union(keys, other)
            }
        }
    }

    fn call(&mut self, callee: &Expr, args: &[Expr]) -> Keys {
        let start = self.tick();
        let mut writes = Writes::default();
        let mut result: Keys = None;
        match callee {
            Expr::Ident(id) => match self.lookup(&id.name) {
                // A fn value held in a local or a parameter.
                Some(_) => {
                    self.use_name(&id.name);
                    writes = Writes::top();
                }
                None => writes.add(&self.named_fn(&[id.name.as_str()])),
            },
            Expr::Path(q) => {
                let segs: Vec<&str> = q.segments.iter().map(|s| s.name.as_str()).collect();
                writes.add(&self.named_fn(&segs));
            }
            Expr::Field { receiver, name, .. } | Expr::Path2 { receiver, name, .. } => {
                result = self.expr(receiver);
                if self.p.fn_fields.contains(&name.name) {
                    writes = Writes::top();
                } else {
                    writes.add(&self.callee(Callee::Method(name.name.clone())));
                }
                // A method of a field's value (a form's `set`, a
                // child's) may change what the field holds.
                if let Expr::Field { name: f, .. } = receiver.as_ref() {
                    writes.add(&Writes::field(&f.name));
                }
            }
            other => {
                self.expr(other);
                writes = Writes::top();
            }
        }
        for a in args {
            let keys = self.expr(a);
            // A local passed on is its own binding's use, inside the
            // call: that binding answers for it.
            if let (Some(keys), true, false) = (&keys, self.summary.is_some(), matches!(a, Expr::Ident(_))) {
                if writes.touches(keys) {
                    let span = a.span();
                    let name = keys.iter().next().cloned().unwrap_or_default();
                    self.args.push((Site::Arg(span.start.0, span.end.0), name));
                }
            }
            result = union(result, keys);
        }
        self.write(writes, Some(start));
        result
    }

    /// A call of a free fn or a stdlib leaf by its path.
    fn named_fn(&mut self, segs: &[&str]) -> Writes {
        use hale_types::stdlib_surface::{effects_for, parks_on_async_io, EffectSet};
        if parks_on_async_io(segs)
            || effects_for(segs).is_some_and(|e| !e.is_unclassified() && e.contains(EffectSet::PUBLISH))
        {
            return Writes::top();
        }
        let last = segs.last().copied().unwrap_or_default();
        if segs.first() == Some(&"std") {
            return Writes::default();
        }
        self.callee(Callee::Fn(last.to_string()))
    }

    fn callee(&mut self, c: Callee) -> Writes {
        let mut w = Writes::default();
        if let Some(summary) = self.summary {
            for j in self.p.resolve(&c) {
                w.add(&summary[j]);
            }
        }
        self.callees.insert(c);
        w
    }

    /// The instances whose value some use reads after a write that may
    /// change it.
    fn flagged(&self) -> Vec<(Site, String)> {
        let mut out = self.args.clone();
        for inst in &self.insts {
            let hit = self.writes.iter().any(|w| {
                w.at > inst.at
                    && w.writes.touches(&inst.keys)
                    && self.uses[inst.var].iter().any(|&u| {
                        u >= w.start
                            || self.loops.iter().any(|&(ls, le)| {
                                ls > self.declared[inst.var] && ls <= u.min(w.start) && u.max(w.at) <= le
                            })
                    })
            });
            if hit {
                out.push((inst.site, inst.name.clone()));
            }
        }
        out
    }
}

impl<'ctx, 'p> crate::codegen::Cx<'ctx, 'p> {
    /// An owned copy, in the frame's arena, of `val`, a value a binding
    /// would otherwise view in its place (`OwnedViews`): every String
    /// and Bytes copied even from the frame's own arena (a `run()` body
    /// allocates from the locus's), every record, tuple, array and
    /// payload-enum block fresh (`emit_owned_deep_copy`). Any other
    /// value is its own.
    pub(crate) fn own_viewed_value(
        &mut self,
        val: inkwell::values::BasicValueEnum<'ctx>,
        ty: &crate::codegen::CodegenTy,
    ) -> Result<inkwell::values::BasicValueEnum<'ctx>, crate::codegen::CodegenError> {
        use crate::codegen::CodegenTy;
        match ty {
            CodegenTy::String
            | CodegenTy::Bytes
            | CodegenTy::TypeRef(_)
            | CodegenTy::Tuple(_)
            | CodegenTy::Array(_, _)
            | CodegenTy::Enum(_) => {
                let dest = self.current_arena_ptr()?;
                self.emit_owned_deep_copy(val, ty, dest)
            }
            _ => Ok(val),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flagged(src: &str) -> Vec<String> {
        let program = hale_syntax::parse_source(src).expect("parses");
        let mut names: Vec<String> = owned_view_sites(&program).into_iter().map(|(_, n)| n).collect();
        names.sort();
        names
    }

    const LOCUS: &str = r#"
        type F { s: String = ""; n: Int = 0; }
        locus H {
            params { f: F = F { }; g: F = F { }; count: Int = 0; }
            fn put(t: String) { self.f = F { s: t }; }
            fn bump() { self.count = self.count + 1; }
            fn via() { self.put("x"); }
            fn body() { BODY }
        }
    "#;

    fn with(body: &str) -> Vec<String> {
        flagged(&LOCUS.replace("BODY", body))
    }

    #[test]
    fn a_local_held_across_a_replacing_call_is_owned() {
        assert_eq!(with(r#"let s = self.f.s; self.put("n"); println(s);"#), ["s"]);
        assert_eq!(with(r#"let s = self.f.s; self.via(); println(s);"#), ["s"]);
    }

    #[test]
    fn a_local_with_no_replacing_call_after_it_stays_a_view() {
        assert!(with(r#"let s = self.f.s; println(s); self.put("n");"#).is_empty());
        assert!(with(r#"let s = self.f.s; self.bump(); println(s);"#).is_empty());
        assert!(with(r#"let s = self.g.s; self.put("n"); println(s);"#).is_empty());
    }

    #[test]
    fn a_direct_store_reaches_a_later_use() {
        assert_eq!(with(r#"let s = self.f.s; self.f.s = "abc"; println(s);"#), ["s"]);
    }

    #[test]
    fn a_local_passed_to_the_writing_call_is_owned() {
        assert_eq!(with(r#"let s = self.f.s; self.put(s);"#), ["s"]);
    }

    #[test]
    fn a_loop_carries_a_later_write_to_an_earlier_use() {
        assert_eq!(
            with(r#"let s = self.f.s; let mut i = 0; while i < 2 { println(s); self.put("n"); i = i + 1; }"#),
            ["s"]
        );
        // Bound inside the loop: each iteration's binding is fresh.
        assert!(with(r#"let mut i = 0; while i < 2 { let s = self.f.s; println(s); self.put("n"); i = i + 1; }"#)
            .is_empty());
    }

    #[test]
    fn an_alias_of_a_view_is_a_view() {
        assert_eq!(with(r#"let s = self.f.s; let t = s; self.put("n"); println(t);"#), ["t"]);
    }

    #[test]
    fn a_field_passed_to_a_call_that_replaces_it_is_owned() {
        assert_eq!(with(r#"self.put(self.f.s);"#), ["f"]);
        assert!(with(r#"self.put(self.g.s);"#).is_empty());
    }

    #[test]
    fn a_match_payload_and_a_for_element_are_views() {
        assert_eq!(with(r#"for x in self.f.s { self.put("n"); println(x); }"#), ["x"]);
        // The next element is read after the write.
        assert_eq!(with(r#"for x in self.f.s { println(x); self.put("n"); }"#), ["x"]);
        assert!(with(r#"for x in self.f.s { println(x); self.bump(); }"#).is_empty());
        let src = r#"
            type E = enum { A(String), B };
            locus H {
                params { e: E = E::B; }
                fn set(t: String) { self.e = E::A(t); }
                fn body() {
                    match self.e { E::A(s) -> { self.set("n"); println(s); }, E::B -> { }, }
                }
            }
        "#;
        assert_eq!(flagged(src), ["s"]);
    }

    #[test]
    fn a_value_carried_out_of_a_match_if_or_block_is_owned() {
        let src = r#"
            type E = enum { A(String), B };
            locus H {
                params { e: E = E::B; flag: Bool = true; }
                fn clear() { self.e = E::B; }
                fn matched() -> String {
                    let held = match self.e { E::A(s) -> s, E::B -> "", };
                    self.clear();
                    return held;
                }
                fn cond() -> String {
                    let m = match self.e { E::A(s) -> s, E::B -> "", };
                    let held = if self.flag { m } else { "" };
                    self.clear();
                    return held;
                }
                fn block() -> String {
                    let held = { let t = match self.e { E::A(s) -> s, E::B -> "", }; t };
                    self.clear();
                    return held;
                }
                fn early() -> String {
                    let held = match self.e { E::A(s) -> s, E::B -> "", };
                    let out = held + "!";
                    self.clear();
                    return out;
                }
            }
        "#;
        let got = owned_view_sites(&hale_syntax::parse_source(src).expect("parses"));
        let lets = got.iter().filter(|(site, _)| matches!(site, Site::Node(_))).count();
        assert_eq!(lets, 3, "{got:?}");
    }

    #[test]
    fn a_literal_or_a_call_holding_a_view_is_a_view() {
        assert_eq!(with(r#"let t = (self.f.s, 1); self.put("n"); println(t.0);"#), ["t"]);
        assert_eq!(with(r#"let r = F { s: self.f.s }; self.put("n"); println(r.s);"#), ["r"]);
        assert_eq!(with(r#"let t = std::str::trim(self.f.s); self.put("n"); println(t);"#), ["t"]);
        // Concatenation is a fresh value.
        assert!(with(r#"let t = self.f.s + "!"; self.put("n"); println(t);"#).is_empty());
    }

    #[test]
    fn an_outer_local_rebound_in_a_loop_carries_round_the_back_edge() {
        assert_eq!(
            with(r#"let mut held = ""; let mut i = 0; while i < 2 { println(held); held = self.f.s; self.put("n"); i = i + 1; }"#),
            ["held"]
        );
        // Rebound after the write: the next iteration reads it before
        // the next write.
        assert!(with(r#"let mut held = ""; let mut i = 0; while i < 2 { println(held); self.put("n"); held = self.f.s; i = i + 1; }"#)
            .is_empty());
        // A for, and a loop nested in another.
        assert_eq!(
            with(r#"let mut held = ""; for k in [1, 2] { println(held); held = self.f.s; self.put("n"); }"#),
            ["held"]
        );
        assert_eq!(
            with(r#"let mut held = ""; let mut i = 0; while i < 2 { let mut j = 0; while j < 2 { println(held); held = self.f.s; self.put("n"); j = j + 1; } i = i + 1; }"#),
            ["held"]
        );
    }

    #[test]
    fn a_returned_view_is_owned() {
        // The tail and a `return` alike; a fresh value is not.
        assert_eq!(flagged(&LOCUS.replace("fn body() { BODY }", "fn name() -> String { self.f.s }")), ["f"]);
        assert_eq!(with(r#"let s = self.f.s; return s;"#), ["f"]);
        assert!(with(r#"return self.f.s + "!";"#).is_empty());
    }

    #[test]
    fn a_publish_or_a_fn_value_may_write_anything() {
        assert_eq!(with(r#"let s = self.g.s; yield; println(s);"#), ["s"]);
    }
}
