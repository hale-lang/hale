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
//! their field names, its keys:
//!
//! - a field read views its field and what it is read from, when that
//!   views a place or is a handle (`self`, a handle parameter or local,
//!   a locus literal, a call declared to return one, any field read); a
//!   field of a record value the frame holds itself views nothing a
//!   write can replace (a write through a local root replaces the
//!   field's pointer, GH #993);
//! - a `match` arm's payload, an `if`, block or `or` result, and a
//!   tuple, array or record literal view what they carry out (a
//!   literal's binding copies the parts that view a place where the
//!   literal is built, not the literal whole);
//! - an enum constructor views its arguments and a form's read
//!   (`self.v.get(i)`) its cell; any other call's result is copied out
//!   of the callee's frame (a method's into the caller's arena, a
//!   stdlib leaf's built fresh) and views nothing.
//!
//! A binding needs its own copy when some use of it comes after (or as
//! an argument of) a write that may change one of its keys, and so
//! does an operand held while a later sibling is evaluated (an earlier
//! argument, a receiver, a binary operator's left side, a literal's
//! earlier field) when that sibling's evaluation may change one:
//!
//! - an assignment `x.f... = v` (any root) writes key `f`;
//! - a call writes what its callee writes, transitively: a locus
//!   method by name, over every locus declaring one (the receiver's
//!   type is not known here), a free fn by name, and a method called
//!   on a field (`self.v.set(..)`, a form) writes that field too;
//! - a publish, an explicit `yield` and a stdlib leaf that publishes
//!   or parks on an `async_io` pool (where another delivery to the same
//!   locus runs meanwhile) write what any bus handler writes;
//! - a call the pass cannot positively identify — through a fn value
//!   (a local, a parameter, a field whose type is a fn or cannot be
//!   classified), of a name no body declares, a recovery statement —
//!   may write anything. Unknown never means "writes nothing".
//!
//! Every question about a type (a handle? a fn value? a form?) is
//! answered by `Bodies::classify` over the resolved type — aliases
//! followed through imports and minted names — and a type it cannot
//! place answers the way that costs a copy.
//!
//! In a loop, a write anywhere in the body reaches a use anywhere in it
//! when the variable was declared before the loop, wherever in the
//! loop the instance holding the value was assigned. Field names stand
//! in for fields of any type, so the answer only errs toward a copy.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use hale_syntax::ast::*;

/// The binding sites lowering binds an owned copy at. A `match` is
/// named by its span (a `MatchStmt` has no id); every other site by its
/// snapshot id.
#[derive(Debug, Clone, Default)]
pub struct OwnedViews {
    sites: HashSet<Site>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Site {
    /// A `let`, a tuple `let`, a bare-local `=` or a `for`, by id.
    Node(u32),
    /// A `match`'s payload bindings, by the match's span.
    Match(u32, u32),
    /// A call's argument or receiver read from a place the callee may
    /// replace (the callee reads its parameter after), by its span. An
    /// argument is passed by reference — a callee may write a record
    /// through its parameter, and the caller sees it — so only an
    /// immutable leaf (a String, a Bytes) is passed as a copy.
    Arg(u32, u32),
    /// A value position — a literal's part, a binary operator's or an
    /// index's operand — read from a place a later operand may replace:
    /// copied whole where it is evaluated.
    Part(u32, u32),
}

impl OwnedViews {
    /// Run the pass over every body of `programs` (lowering's merged
    /// program); `renames` is the build's import table (a qualified path
    /// to the declaration it names).
    pub fn of(programs: &[&Program], renames: &BTreeMap<Vec<String>, String>) -> Self {
        let mut out = OwnedViews::default();
        for (site, _) in owned_view_sites_in(programs, renames) {
            out.sites.insert(site);
        }
        out
    }

    /// Does the `let`, tuple `let`, bare-local `=` or `for` with this id
    /// bind an owned copy?
    pub fn node(&self, id: NodeId) -> bool {
        !id.is_none() && self.sites.contains(&Site::Node(id.0))
    }

    /// Do this match's payload bindings bind owned copies?
    pub fn match_arms(&self, m: &MatchStmt) -> bool {
        self.sites.contains(&Site::Match(m.span.start.0, m.span.end.0))
    }

    /// Is `e` an operand read from a place a later write may replace,
    /// copied where it is evaluated? `Some(true)`: a call's argument or
    /// receiver, copied only when it is an immutable leaf (String,
    /// Bytes) — a record passed by reference stays the caller's, which
    /// the callee may write through; `Some(false)`: a value position,
    /// copied whole.
    pub fn operand(&self, e: &Expr) -> Option<bool> {
        if self.sites.is_empty() {
            return None;
        }
        let span = e.span();
        if self.sites.contains(&Site::Part(span.start.0, span.end.0)) {
            Some(false)
        } else if self.sites.contains(&Site::Arg(span.start.0, span.end.0)) {
            Some(true)
        } else {
            None
        }
    }
}

/// What a body may write: every field name, or these.
#[derive(Debug, Clone, Default, PartialEq)]
struct Writes {
    top: bool,
    /// Field names by their `Bodies::field_id`, a bit each: the
    /// summaries' fixpoint unions these for every call edge of every
    /// body, round after round.
    bits: Vec<u64>,
}

impl Writes {
    fn top() -> Self {
        Writes { top: true, bits: Vec::new() }
    }
    fn field(id: usize) -> Self {
        let mut w = Writes::default();
        let (i, b) = (id / 64, id % 64);
        w.bits.resize(i + 1, 0);
        w.bits[i] |= 1 << b;
        w
    }
    fn add(&mut self, o: &Writes) {
        self.top |= o.top;
        if self.top {
            self.bits.clear();
            return;
        }
        if self.bits.len() < o.bits.len() {
            self.bits.resize(o.bits.len(), 0);
        }
        for (a, b) in self.bits.iter_mut().zip(&o.bits) {
            *a |= *b;
        }
    }
    fn is_empty(&self) -> bool {
        !self.top && self.bits.iter().all(|w| *w == 0)
    }
    fn has(&self, id: usize) -> bool {
        self.bits.get(id / 64).is_some_and(|w| w & (1 << (id % 64)) != 0)
    }
    fn touches(&self, keys: &BTreeSet<String>, p: &Bodies<'_>) -> bool {
        self.top || keys.iter().any(|k| p.known_field(k).is_some_and(|id| self.has(id)))
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
    /// Every bus handler: what a publish, a `yield` or a park may run.
    Deliveries,
}

/// Every body of the program, with the summaries the pass reads.
struct Bodies<'a> {
    bodies: Vec<(&'a [Param], &'a Block)>,
    /// Method name → the summaries' index of each body declaring it.
    methods: BTreeMap<String, Vec<usize>>,
    fns: BTreeMap<String, Vec<usize>>,
    /// Params fields by name, each declared type (`None`: inferred from
    /// its default), classified once the declarations are in.
    field_types: Vec<(String, Option<&'a TypeExpr>)>,
    /// Field names a call `x.name(..)` may call as a value: a fn type, or
    /// a type this pass cannot classify.
    fn_fields: BTreeSet<String>,
    /// Bus handler names: a publish, a `yield` or a park may run any
    /// of them before it returns.
    handlers: BTreeSet<String>,
    /// `@form` loci, and every params field naming one (by its type's
    /// last segment): a form's read (`self.v.get(i)`) may hand back a
    /// view of its cell, which the form's own writes retire.
    form_loci: BTreeSet<String>,
    /// Records and enums, by every name they answer to: values the frame
    /// holds, not handles.
    records: BTreeSet<String>,
    enums: BTreeSet<String>,
    /// Type aliases, by their normalized names: classified by what they
    /// name, never by their spelling.
    aliases: BTreeMap<String, &'a TypeExpr>,
    /// The build's import table: `lib::Kid` names the declaration minted
    /// for the library's `Kid`.
    renames: BTreeMap<Vec<String>, String>,
    /// Every enum variant's name: a call of one constructs a value that
    /// holds its arguments.
    variants: BTreeSet<String>,
    form_fields: BTreeSet<String>,
    /// Field names every declaration of which holds an `@form` locus.
    form_only_fields: BTreeSet<String>,
    /// Loci, interfaces and perspectives: a value of one of these is a
    /// handle, whose field reads view the locus's storage.
    loci: BTreeSet<String>,
    /// Every fn and method name with each body's declared return type
    /// (`None`: returns nothing), classified once the declarations are
    /// in.
    returns: Vec<(String, Option<&'a TypeExpr>)>,
    /// Fn and method names some body of which may return a handle.
    handle_fns: BTreeSet<String>,
    /// Every field name a write names, numbered for `Writes`' bits.
    field_ids: std::cell::RefCell<BTreeMap<String, usize>>,
}

impl Bodies<'_> {
    fn field_id(&self, name: &str) -> usize {
        let mut ids = self.field_ids.borrow_mut();
        let n = ids.len();
        *ids.entry(name.to_string()).or_insert(n)
    }
    fn known_field(&self, name: &str) -> Option<usize> {
        self.field_ids.borrow().get(name).copied()
    }
}

/// The names a declaration answers to. An imported seed's declaration
/// is minted `__lib_<alias>_<stem>_<Name>`, and a use spells it either
/// that way (inside the seed, after the mangle), through the import
/// table (`lib::Name`, `Bodies::path_name`), or — in a body the mangle
/// did not reach — as `Name`: the minted name answers to every suffix
/// after an underscore too. A suffix shared with an unrelated
/// declaration only makes the answer err toward a copy.
fn decl_names(name: &str) -> Vec<String> {
    let mut out = vec![name.to_string()];
    if name.starts_with("__lib_") {
        for (i, _) in name.match_indices('_') {
            if i + 1 < name.len() && !name[i + 1..].starts_with('_') {
                out.push(name[i + 1..].to_string());
            }
        }
    }
    out
}

impl<'a> Bodies<'a> {
    /// The declaration a path names, by its normalized name: a qualified
    /// path through the import table (or the stdlib's), a bare name as
    /// written.
    fn path_name(&self, segs: &[Ident]) -> Option<String> {
        let last = segs.last()?;
        if segs.len() > 1 {
            let key: Vec<String> = segs.iter().map(|s| s.name.clone()).collect();
            let refs: Vec<&str> = key.iter().map(|s| s.as_str()).collect();
            if let Some(m) = self.renames.get(&key) {
                return Some(m.clone());
            }
            if let Some(m) = crate::ownership::stdlib_mangled_for_path(&refs) {
                return Some(m.to_string());
            }
        }
        Some(last.name.clone())
    }

    /// Does the call path name an enum variant (`E::A`, `lib::E::A`)?
    fn is_ctor(&self, q: &QualifiedName) -> bool {
        q.segments.len() > 1 && q.segments.last().is_some_and(|s| self.variants.contains(&s.name))
    }

    /// The one place a type is classified: `ty` with its aliases
    /// followed (`type Child = Kid;`, through any import or minted
    /// name), a mode projection taken off. Every question the pass asks
    /// of a type — a handle? a fn value? a form? — reads this.
    /// A type it cannot place (a generic parameter, a name it does not
    /// know) is `Unknown`, which every question answers the way that
    /// costs a copy rather than a use after a retire.
    fn classify(&self, ty: &TypeExpr) -> Class {
        let mut cur = ty;
        for _ in 0..16 {
            match cur {
                TypeExpr::Projection { inner, .. } => cur = inner,
                TypeExpr::Named { path, .. } => {
                    let Some(name) = self.path_name(&path.segments) else { return Class::Unknown };
                    if let Some(next) = self.aliases.get(&name) {
                        cur = next;
                        continue;
                    }
                    return self.classify_name(&name);
                }
                TypeExpr::Primitive(..) => return Class::Value,
                TypeExpr::Tuple(..) | TypeExpr::Array { .. } | TypeExpr::Bounded { .. } => return Class::Value,
                TypeExpr::Function { .. } => return Class::FnValue,
                TypeExpr::Perspective { .. } => return Class::Handle,
            }
        }
        Class::Unknown
    }

    /// A declared name's class (aliases already followed).
    fn classify_name(&self, name: &str) -> Class {
        if self.form_loci.contains(name) {
            Class::Form
        } else if self.loci.contains(name) {
            Class::Handle
        } else if self.records.contains(name) || self.enums.contains(name) {
            Class::Value
        } else {
            Class::Unknown
        }
    }

    /// The class a literal or constructor path names, its aliases
    /// followed.
    fn classify_path(&self, path: &QualifiedName) -> Class {
        let Some(name) = self.path_name(&path.segments) else { return Class::Unknown };
        match self.aliases.get(&name) {
            Some(t) => self.classify(t),
            None => self.classify_name(&name),
        }
    }
}

/// The method names of the `@form` loci, synthesized by lowering (no
/// body): what they write is the field they are called on.
/// The bare `bounded[T; N]` intrinsics, whose first argument is the
/// bounded field they write (`push(p.path_vals, v)`).
const BOUNDED_OPS: &[&str] = &["count", "clear", "truncate", "push", "at", "pop"];

const FORM_OPS: &[&str] = &[
    "get", "set", "pop", "push", "remove", "insert", "contains", "has", "len", "is_empty", "is_full", "key_at",
    "entry_at", "sort", "bump", "put", "clear",
];

/// What a call's target is, as far as the pass can tell.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    /// A fn or method of the program, with a body and so a summary.
    Fn(String),
    Method(String),
    /// A builtin or stdlib leaf: writes nothing of the program's (or
    /// what the bus handlers write, when it publishes or parks).
    Std,
    /// An enum variant: builds a value holding its arguments.
    Ctor,
    /// A form's synthesized method.
    FormOp,
    /// A bare `bounded[T; N]` intrinsic: writes its first argument.
    Bounded,
    /// Nothing the pass knows: may write anything, may return a handle.
    Unknown,
}

impl Bodies<'_> {
    /// The target of a call through `callee` (not a local fn value);
    /// `on_form`: its receiver is known to be an `@form` locus, whose
    /// synthesized methods take a name before any method of another
    /// locus that shares it.
    fn target(&self, callee: &Expr, on_form: bool) -> Target {
        use crate::builtin_sigs::bare_builtin_sig;
        use crate::stdlib_surface::{builtin_effects, row};
        match callee {
            Expr::Ident(id) => {
                let n = id.name.as_str();
                if self.fns.contains_key(n) {
                    Target::Fn(n.to_string())
                } else if BOUNDED_OPS.contains(&n) {
                    Target::Bounded
                } else if self.variants.contains(n) {
                    Target::Ctor
                } else if bare_builtin_sig(n).is_some() || builtin_effects(n).is_some() || row(&[n]).is_some() {
                    Target::Std
                } else {
                    Target::Unknown
                }
            }
            Expr::Path(q) => {
                let segs: Vec<&str> = q.segments.iter().map(|s| s.name.as_str()).collect();
                if self.is_ctor(q) {
                    return Target::Ctor;
                }
                if segs.first() == Some(&"std") || row(&segs).is_some() {
                    return Target::Std;
                }
                match self.path_name(&q.segments) {
                    Some(n) if self.fns.contains_key(&n) => Target::Fn(n),
                    // `Type::method(..)`
                    _ if self.methods.contains_key(*segs.last().unwrap_or(&"")) => {
                        Target::Method(segs.last().unwrap_or(&"").to_string())
                    }
                    _ => Target::Unknown,
                }
            }
            Expr::Field { name, .. } | Expr::Path2 { name, .. } => {
                if on_form && FORM_OPS.contains(&name.name.as_str()) {
                    Target::FormOp
                } else if self.fn_fields.contains(&name.name) {
                    // A fn-valued (or unclassified) field of that name: a
                    // value call, whatever methods share the name.
                    Target::Unknown
                } else if self.methods.contains_key(&name.name) {
                    Target::Method(name.name.clone())
                } else {
                    // A form's synthesized method only on a receiver known
                    // to hold a form: by its name alone it may be a record's
                    // fn-valued member, or anything else.
                    Target::Unknown
                }
            }
            _ => Target::Unknown,
        }
    }

    /// May a call of `t` return a handle?
    fn returns_handle(&self, t: &Target) -> bool {
        match t {
            Target::Fn(n) | Target::Method(n) => self.handle_fns.contains(n),
            Target::Std | Target::Ctor | Target::FormOp | Target::Bounded => false,
            Target::Unknown => true,
        }
    }
}

/// What a type is, as the pass needs to know it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    /// A scalar, String or Bytes, record, enum, tuple or array: a value
    /// the frame holds (its fields view nothing another write replaces).
    Value,
    /// A locus, an interface, a perspective: its field reads view the
    /// locus's storage.
    Handle,
    /// An `@form` locus: a handle whose reads may hand back a cell.
    Form,
    /// A fn type: calling one may run anything.
    FnValue,
    /// A type the pass cannot place: treated as a handle, and called as
    /// a value it may run anything.
    Unknown,
}

impl Class {
    fn may_be_handle(self) -> bool {
        matches!(self, Class::Handle | Class::Form | Class::Unknown)
    }
    fn may_be_fn(self) -> bool {
        matches!(self, Class::FnValue | Class::Unknown)
    }
}

fn collect<'a>(items: &'a [TopDecl], p: &mut Bodies<'a>) {
    for item in items {
        match item {
            TopDecl::Fn(f) => {
                // An imported seed's fn is minted under a mangled name and
                // called by its own (`lib::name`, `name`).
                for n in decl_names(&f.name.name) {
                    p.fns.entry(n.clone()).or_default().push(p.bodies.len());
                    p.returns.push((n, f.ret.as_ref()));
                }
                p.bodies.push((&f.params, &f.body));
            }
            TopDecl::Interface(i) => {
                p.loci.extend(decl_names(&i.name.name));
            }
            TopDecl::Locus(l) => {
                p.loci.extend(decl_names(&l.name.name));
                if l.form.is_some() {
                    p.form_loci.extend(decl_names(&l.name.name));
                }
                for m in &l.members {
                    match m {
                        LocusMember::Fn(f) => {
                            p.methods.entry(f.name.name.clone()).or_default().push(p.bodies.len());
                            p.bodies.push((&f.params, &f.body));
                            p.returns.push((f.name.name.clone(), f.ret.as_ref()));
                        }
                        LocusMember::Lifecycle(d) => p.bodies.push((&d.params, &d.body)),
                        LocusMember::Mode(d) => p.bodies.push((&d.params, &d.body)),
                        LocusMember::Failure(d) => p.bodies.push((&d.params, &d.body)),
                        LocusMember::Params(pb) => {
                            for d in &pb.params {
                                p.field_types.push((d.name.name.clone(), d.ty.as_ref()));
                            }
                        }
                        LocusMember::Bus(b) => {
                            for bm in &b.members {
                                if let BusMember::Subscribe { handler, .. } = bm {
                                    p.handlers.insert(handler.name.clone());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            TopDecl::Perspective(d) => {
                p.loci.extend(decl_names(&d.name.name));
                for m in &d.members {
                    if let PerspectiveMember::Fn(f) = m {
                        p.methods.entry(f.name.name.clone()).or_default().push(p.bodies.len());
                        p.bodies.push((&f.params, &f.body));
                        p.returns.push((f.name.name.clone(), f.ret.as_ref()));
                    }
                }
            }
            TopDecl::Module(m) => collect(&m.items, p),
            TopDecl::Type(td) => {
                match &td.body {
                    TypeDeclBody::Enum(vs) => {
                        p.variants.extend(vs.iter().map(|v| v.name.name.clone()));
                        p.enums.extend(decl_names(&td.name.name));
                    }
                    TypeDeclBody::Struct(fields) => {
                        p.records.extend(decl_names(&td.name.name));
                        // A record's members are fields a call
                        // `x.name(..)` may run as a value, like a locus's.
                        for f in fields {
                            p.field_types.push((f.name.name.clone(), Some(&f.ty)));
                        }
                    }
                    TypeDeclBody::Scalar(_) => p.records.extend(decl_names(&td.name.name)),
                    TypeDeclBody::Alias(_) => {}
                }
                if let TypeDeclBody::Alias(t) = &td.body {
                    for n in decl_names(&td.name.name) {
                        p.aliases.insert(n, t);
                    }
                }
            }
            _ => {}
        }
    }
}

/// The flagged sites, each with the name it binds (the first name of a
/// tuple `let`, a match's first payload binding): for the tests.
#[cfg(test)]
fn owned_view_sites(program: &Program) -> Vec<(Site, String)> {
    owned_view_sites_with(program, &BTreeMap::new())
}

#[cfg(test)]
fn owned_view_sites_with(program: &Program, renames: &BTreeMap<Vec<String>, String>) -> Vec<(Site, String)> {
    owned_view_sites_in(&[program], renames)
}

fn owned_view_sites_in(programs: &[&Program], renames: &BTreeMap<Vec<String>, String>) -> Vec<(Site, String)> {
    let mut p = Bodies {
        bodies: Vec::new(),
        methods: BTreeMap::new(),
        fns: BTreeMap::new(),
        fn_fields: BTreeSet::new(),
        handlers: BTreeSet::new(),
        form_loci: BTreeSet::new(),
        records: BTreeSet::new(),
        enums: BTreeSet::new(),
        field_types: Vec::new(),
        aliases: BTreeMap::new(),
        renames: renames.clone(),
        variants: BTreeSet::new(),
        form_fields: BTreeSet::new(),
        form_only_fields: BTreeSet::new(),
        loci: BTreeSet::new(),
        returns: Vec::new(),
        handle_fns: BTreeSet::new(),
        field_ids: std::cell::RefCell::new(BTreeMap::new()),
    };
    for program in programs {
        collect(&program.items, &mut p);
    }
    // Every classification below reads `classify`: a fn or method name
    // some body of which may return a handle; a field a call may run as
    // a value; a field holding a form. A field with no declared type
    // (inferred from its default) is unclassified.
    let handle_fns: BTreeSet<String> = p
        .returns
        .iter()
        .filter(|(_, r)| r.is_some_and(|t| p.classify(t).may_be_handle()))
        .map(|(n, _)| n.clone())
        .collect();
    let classes: Vec<(String, Class)> = p
        .field_types
        .iter()
        .map(|(f, t)| (f.clone(), t.map_or(Class::Unknown, |t| p.classify(t))))
        .collect();
    p.handle_fns = handle_fns;
    p.fn_fields = classes.iter().filter(|(_, c)| c.may_be_fn()).map(|(f, _)| f.clone()).collect();
    p.form_fields = classes.iter().filter(|(_, c)| matches!(c, Class::Form | Class::Unknown)).map(|(f, _)| f.clone()).collect();
    let not_form: BTreeSet<&String> = classes.iter().filter(|(_, c)| *c != Class::Form).map(|(f, _)| f).collect();
    p.form_only_fields =
        classes.iter().filter(|(f, c)| *c == Class::Form && !not_form.contains(f)).map(|(f, _)| f.clone()).collect();

    // Each body's own writes and the calls it makes, then the fixpoint.
    let mut direct: Vec<(Writes, BTreeSet<Callee>)> = Vec::with_capacity(p.bodies.len());
    for (params, body) in &p.bodies {
        let mut w = Walk::new(&p, None, params);
        w.block(body);
        direct.push((w.own, w.callees));
    }
    // Each body's callees, resolved once (a method name to every body
    // declaring it), then unioned until nothing changes.
    let edges: Vec<Vec<usize>> = direct
        .iter()
        .map(|(_, callees)| {
            let mut e: Vec<usize> = callees.iter().flat_map(|c| p.resolve(c)).collect();
            e.sort_unstable();
            e.dedup();
            e
        })
        .collect();
    let mut summary: Vec<Writes> = direct.into_iter().map(|(w, _)| w).collect();
    loop {
        let mut changed = false;
        for i in 0..summary.len() {
            if summary[i].top {
                continue;
            }
            let mut next = summary[i].clone();
            for &j in &edges[i] {
                if j != i {
                    next.add(&summary[j]);
                    if next.top {
                        break;
                    }
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
        w.block(body);
        out.extend(w.flagged());
    }
    out
}

impl Bodies<'_> {
    fn resolve(&self, c: &Callee) -> Vec<usize> {
        let m = match c {
            Callee::Method(n) => self.methods.get(n),
            Callee::Fn(n) => self.fns.get(n),
            Callee::Deliveries => {
                return self.handlers.iter().filter_map(|h| self.methods.get(h)).flatten().copied().collect();
            }
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
    /// A literal's binding: the parts that view a place, copied where
    /// the literal is built instead of the literal copied whole.
    parts: Option<Vec<Site>>,
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
    /// Per variable, whether it may hold a handle, whose field reads
    /// view a locus's storage; a record value of the frame's own is
    /// never replaced under a binding of its field.
    handles: Vec<bool>,
    /// The parameters that are handles.
    handle_params: BTreeSet<String>,
    /// Per variable, whether it holds an `@form` locus (bound from a
    /// form's literal or declared as one); and the form parameters.
    forms: Vec<bool>,
    form_params: BTreeSet<String>,
    /// Per variable, where it was declared: a loop that starts after
    /// it carries its value round the back edge, whichever instance of
    /// it holds the value.
    declared: Vec<u32>,
    insts: Vec<Inst>,
    /// Per variable, the positions it is read at.
    uses: Vec<Vec<u32>>,
    writes: Vec<WriteAt>,
    loops: Vec<(u32, u32)>,
    /// What `Callee::Deliveries` writes, once the summaries are in.
    deliveries: Option<Writes>,
    /// Arguments passed to a call that may replace what they view.
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
        let mut handle_params = BTreeSet::new();
        let mut form_params = BTreeSet::new();
        for prm in params {
            top.insert(prm.name.name.clone(), None);
            let class = p.classify(&prm.ty);
            if class.may_be_handle() {
                handle_params.insert(prm.name.name.clone());
            }
            if class == Class::Form {
                form_params.insert(prm.name.name.clone());
            }
        }
        Walk {
            p,
            summary,
            own: Writes::default(),
            callees: BTreeSet::new(),
            pos: 0,
            scopes: vec![top],
            vars: Vec::new(),
            handles: Vec::new(),
            handle_params,
            forms: Vec::new(),
            form_params,
            declared: Vec::new(),
            insts: Vec::new(),
            uses: Vec::new(),
            writes: Vec::new(),
            loops: Vec::new(),
            args: Vec::new(),
            deliveries: None,
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
        self.handles.push(false);
        self.forms.push(false);
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
        self.bind_parts(var, site, name, keys, None);
    }

    fn bind_parts(
        &mut self,
        var: usize,
        site: Site,
        name: &str,
        keys: Option<BTreeSet<String>>,
        parts: Option<Vec<Site>>,
    ) {
        let at = self.tick();
        let Some(keys) = keys else { return };
        // Read out of a place, it may be a child's handle.
        self.handles[var] = true;
        let i = self.insts.len();
        self.insts.push(Inst { var, site, name: name.to_string(), keys, at, parts });
        self.vars[var].push(i);
    }

    /// The keys of `receiver.name`, the receiver's being `keys`: a field
    /// read views its field when the receiver views a place or holds a
    /// handle; a field of a record value the frame holds views nothing
    /// another write can replace (a write through a local root replaces
    /// the field's pointer, GH #993).
    fn field_keys(&self, receiver: &Expr, keys: Keys, name: &Ident) -> Keys {
        if keys.is_none() && !self.is_handle(receiver) {
            return None;
        }
        let mut keys = keys.unwrap_or_default();
        keys.insert(name.name.clone());
        Some(keys)
    }

    /// Is the receiver of the method call `callee` known to be an
    /// `@form` locus: a form-typed field, a form parameter, a local bound
    /// from a form's literal or declared as one?
    fn on_form(&self, callee: &Expr) -> bool {
        let (Expr::Field { receiver, .. } | Expr::Path2 { receiver, .. }) = callee else { return false };
        match receiver.as_ref() {
            Expr::Field { name, .. } => self.p.form_only_fields.contains(&name.name),
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(Some(v)) => self.forms[v],
                Some(None) => self.form_params.contains(&id.name),
                None => false,
            },
            _ => false,
        }
    }

    /// May `e` be a handle? `self`, a handle parameter, a local that
    /// may hold one, a locus literal, a call of a fn or method declared
    /// to return one, and any field read (a field may hold a child).
    fn is_handle(&self, e: &Expr) -> bool {
        match e {
            Expr::KwSelf(_) => true,
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(Some(v)) => self.handles[v],
                Some(None) => self.handle_params.contains(&id.name),
                None => false,
            },
            Expr::Struct { path, .. } => self.p.classify_path(path).may_be_handle(),
            Expr::Field { .. } | Expr::Index { .. } => true,
            Expr::Call { callee, .. } => match callee.as_ref() {
                // A fn value held in a local or a parameter.
                Expr::Ident(id) if self.lookup(&id.name).is_some() => true,
                c => self.p.returns_handle(&self.p.target(c, self.on_form(c))),
            },
            Expr::If(_) | Expr::Match(_) | Expr::Block(_) | Expr::Or { .. } => true,
            _ => false,
        }
    }

    /// For a tuple, array or record literal, the parts that view a
    /// place (nested literals walked through), by span: an owned copy
    /// of each where the literal is built leaves the literal owning
    /// what it holds, where a copy of the whole would also copy every
    /// scalar and fresh part. `None` for any other expression.
    fn literal_parts(&self, e: &Expr) -> Option<Vec<Site>> {
        fn walk(w: &Walk<'_, '_>, e: &Expr, out: &mut Vec<Site>) {
            match e {
                Expr::Tuple(es, _) | Expr::Array(es, _) => es.iter().for_each(|x| walk(w, x, out)),
                Expr::Struct { inits, .. } => inits.iter().for_each(|i| walk(w, &i.value, out)),
                Expr::ArrayRepeat { val, .. } => walk(w, val, out),
                // What these carry out is known only inside them: the
                // part is copied whole.
                Expr::Match(_) | Expr::If(_) | Expr::Block(_) | Expr::Or { .. } => {
                    let span = e.span();
                    out.push(Site::Part(span.start.0, span.end.0));
                }
                _ => {
                    if w.part_keys(e).is_some() {
                        let span = e.span();
                        out.push(Site::Part(span.start.0, span.end.0));
                    }
                }
            }
        }
        if !matches!(e, Expr::Tuple(..) | Expr::Array(..) | Expr::Struct { .. } | Expr::ArrayRepeat { .. }) {
            return None;
        }
        let mut out = Vec::new();
        walk(self, e, &mut out);
        Some(out)
    }

    /// The keys of a field read, a local or an enum constructor, read
    /// without walking (`expr` has walked it already).
    fn part_keys(&self, e: &Expr) -> Keys {
        match e {
            Expr::Field { receiver, name, .. } => self.field_keys(receiver, self.part_keys(receiver), name),
            Expr::Index { receiver, .. } => self.part_keys(receiver),
            Expr::Ident(id) => match self.lookup(&id.name) {
                Some(Some(v)) => self.var_keys(v),
                _ => None,
            },
            Expr::Call { callee, args, .. } => match callee.as_ref() {
                Expr::Path(q) if self.p.is_ctor(q) =>
                {
                    args.iter().fold(None, |acc, a| union(acc, self.part_keys(a)))
                }
                _ => None,
            },
            _ => None,
        }
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
            Stmt::Let { name, ty, value, id, .. } => {
                let keys = self.expr(value);
                let parts = self.literal_parts(value);
                // A binding declared as a handle is one, whatever the
                // value's own spelling says.
                let handle = self.is_handle(value) || ty.as_ref().is_some_and(|t| self.p.classify(t).may_be_handle());
                let form = match (ty, value) {
                    (Some(t), _) => self.p.classify(t) == Class::Form,
                    (None, Expr::Struct { path, .. }) => self.p.classify_path(path) == Class::Form,
                    _ => false,
                };
                let v = self.declare(&name.name);
                self.handles[v] = handle;
                self.forms[v] = form;
                self.bind_parts(v, Site::Node(id.0), &name.name, keys, parts);
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
                        if self.is_handle(value) {
                            self.handles[v] = true;
                        }
                        let parts = self.literal_parts(value);
                        self.bind_parts(v, Site::Node(id.0), &target.head.name, keys, parts);
                    }
                } else if let Some(f) = first {
                    let id = self.p.field_id(f);
                    self.write(Writes::field(id), None);
                } else {
                    // `x[i] = v` on a local or a parameter: the element
                    // a binding of `x[j]` views.
                    self.use_name(&target.head.name);
                    let id = self.p.field_id(&target.head.name);
                    self.write(Writes::field(id), None);
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
                    self.expr(e);
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
                let w = self.deliveries();
                self.write(w, None);
            }
            Stmt::Yield(_) => {
                let w = self.deliveries();
                self.write(w, None);
            }
            Stmt::Reperspective { .. } => self.write(Writes::top(), None),
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
                self.operands(&[left, right]);
                None
            }
            Expr::Unary { operand, .. } => {
                self.expr(operand);
                None
            }
            Expr::Call { callee, args, .. } => self.call(callee, args),
            Expr::Field { receiver, name, .. } => {
                let keys = self.expr(receiver);
                self.field_keys(receiver, keys, name)
            }
            Expr::Path2 { receiver, .. } => {
                self.expr(receiver);
                None
            }
            Expr::Index { receiver, index, .. } => {
                let mut keys = self.operands(&[receiver, index]);
                keys.swap_remove(0)
            }
            Expr::Tuple(es, _) | Expr::Array(es, _) => {
                let ops: Vec<&Expr> = es.iter().collect();
                self.operands(&ops).into_iter().fold(None, union)
            }
            Expr::Struct { inits, .. } => {
                let ops: Vec<&Expr> = inits.iter().map(|i| &i.value).collect();
                self.operands(&ops).into_iter().fold(None, union)
            }
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
        let mut ctor = false;
        let mut form_op = false;
        let mut receiver: Option<&Expr> = None;
        // A target the pass cannot positively identify may write
        // anything: missed precision costs a copy, never a use after a
        // retire.
        match callee {
            Expr::Ident(id) if self.lookup(&id.name).is_some() => {
                // A fn value held in a local or a parameter.
                self.use_name(&id.name);
                writes = Writes::top();
            }
            Expr::Ident(_) | Expr::Path(_) | Expr::Field { .. } | Expr::Path2 { .. } => {
                if let Expr::Field { receiver: r, .. } | Expr::Path2 { receiver: r, .. } = callee {
                    receiver = Some(r);
                    // A method of a field's value (a form's `set`, a
                    // child's) may change what the field holds.
                    if let Expr::Field { name: f, .. } = r.as_ref() {
                        writes.add(&Writes::field(self.p.field_id(&f.name)));
                    }
                }
                match self.p.target(callee, self.on_form(callee)) {
                    Target::Fn(n) => writes.add(&self.callee(Callee::Fn(n))),
                    Target::Method(n) => writes.add(&self.callee(Callee::Method(n))),
                    Target::Std => {
                        let segs: Vec<&str> = match callee {
                            Expr::Ident(id) => vec![id.name.as_str()],
                            Expr::Path(q) => q.segments.iter().map(|s| s.name.as_str()).collect(),
                            _ => Vec::new(),
                        };
                        writes.add(&self.std_leaf(&segs));
                    }
                    // An enum constructor holds its arguments; a fn's or a
                    // stdlib leaf's result is copied out of its frame.
                    Target::Ctor => ctor = true,
                    Target::FormOp => form_op = true,
                    Target::Bounded => {
                        // The bounded field it is handed.
                        if let Some(Expr::Field { name: f, .. }) = args.first() {
                            writes.add(&Writes::field(self.p.field_id(&f.name)));
                        }
                    }
                    Target::Unknown => writes = Writes::top(),
                }
            }
            other => {
                self.expr(other);
                writes = Writes::top();
            }
        }
        // The receiver and the arguments, in the order they are
        // evaluated: each is held while the later ones run.
        let mut ops: Vec<&Expr> = Vec::with_capacity(args.len() + 1);
        ops.extend(receiver);
        ops.extend(args.iter());
        let mut keys = self.operands(&ops);
        let recv_keys = if receiver.is_some() { keys.remove(0) } else { None };
        let mut result: Keys = None;
        // A form's read may hand back its cell; any other method's
        // result is copied into the caller's arena.
        if let Some(Expr::Field { name: f, .. }) = receiver {
            if self.p.form_fields.contains(&f.name) {
                result = recv_keys.clone();
            }
        }
        if form_op {
            // A form held in a local or a parameter: its cells are a place
            // of their own, named after it, which its writes (`set`,
            // `remove`, `pop`) retire and its reads may view.
            if let Some(Expr::Ident(id)) = receiver {
                let key = format!("@{}", id.name);
                writes.add(&Writes::field(self.p.field_id(&key)));
                let mut k = BTreeSet::new();
                k.insert(key);
                result = union(result, Some(k));
            }
            result = union(result, recv_keys);
        }
        for (a, keys) in args.iter().zip(keys) {
            // An argument the callee may replace before it reads its
            // parameter. A local passed on is its own binding's use,
            // inside the call: that binding answers for it. A form's
            // synthesized method copies its arguments in before it
            // replaces a cell.
            if form_op {
                continue;
            }
            if let (Some(k), true, false) = (&keys, self.summary.is_some(), matches!(a, Expr::Ident(_))) {
                if writes.touches(k, self.p) {
                    self.flag_operand(a, k, true);
                }
            }
            if ctor {
                result = union(result, keys);
            }
        }
        // A form's synthesized method has consumed its arguments by the
        // time it writes: the write spans no argument.
        self.write(writes, if form_op { None } else { Some(start) });
        result
    }

    /// Walk operands evaluated in order and held until the last is: an
    /// operand that views a place is copied where it is evaluated when a
    /// later operand's evaluation may replace that place (`take(match
    /// self.e { .. }, self.reset())`, `self.f.s + self.renamed()`, a
    /// record literal's fields). The keys of each.
    /// A call's receiver and arguments are operands too: one a later
    /// sibling's evaluation replaces is copied whole, a compound
    /// included — the place it referenced is gone before the callee
    /// starts, so there is nothing for the callee to write through.
    fn operands(&mut self, ops: &[&Expr]) -> Vec<Keys> {
        let first_write = self.writes.len();
        let mut keys = Vec::with_capacity(ops.len());
        let mut ends = Vec::with_capacity(ops.len());
        for op in ops {
            keys.push(self.expr(op));
            ends.push(self.pos);
        }
        if self.summary.is_some() && ops.len() > 1 {
            for i in 0..ops.len() - 1 {
                let Some(k) = &keys[i] else { continue };
                let hit = self.writes[first_write..]
                    .iter()
                    .any(|w| w.at > ends[i] && w.writes.touches(k, self.p));
                if hit {
                    let k = k.clone();
                    // A later sibling replaced the place before the call
                    // begins: there is no live place for a callee to write
                    // through, so even a compound argument is copied whole.
                    self.flag_operand(ops[i], &k, false);
                }
            }
        }
        keys
    }

    /// Copy `a`, an operand that views `keys`, where it is evaluated: a
    /// literal by its parts that view a place, anything else whole.
    fn flag_operand(&mut self, a: &Expr, keys: &BTreeSet<String>, by_ref: bool) {
        let name = keys.iter().next().cloned().unwrap_or_default();
        match self.literal_parts(a) {
            Some(parts) => self.args.extend(parts.into_iter().map(|s| (s, name.clone()))),
            None => {
                let span = a.span();
                let site = if by_ref { Site::Arg(span.start.0, span.end.0) } else { Site::Part(span.start.0, span.end.0) };
                self.args.push((site, name));
            }
        }
    }

    /// What a delivery the bus runs before returning may write: every
    /// handler's writes.
    fn deliveries(&mut self) -> Writes {
        if let Some(w) = &self.deliveries {
            return w.clone();
        }
        let w = self.callee(Callee::Deliveries);
        if self.summary.is_some() {
            self.deliveries = Some(w.clone());
        }
        w
    }

    /// What a builtin or stdlib leaf writes: what the bus handlers
    /// write when it publishes or parks on an `async_io` pool, else
    /// nothing of the program's.
    fn std_leaf(&mut self, segs: &[&str]) -> Writes {
        use crate::stdlib_surface::{effects_for, parks_on_async_io, EffectSet};
        if parks_on_async_io(segs)
            || effects_for(segs).is_some_and(|e| !e.is_unclassified() && e.contains(EffectSet::PUBLISH))
        {
            return self.deliveries();
        }
        Writes::default()
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
                    && w.writes.touches(&inst.keys, self.p)
                    && self.uses[inst.var].iter().any(|&u| {
                        u >= w.start
                            || self.loops.iter().any(|&(ls, le)| {
                                ls > self.declared[inst.var] && ls <= u.min(w.start) && u.max(w.at) <= le
                            })
                    })
            });
            if hit {
                match &inst.parts {
                    Some(parts) => out.extend(parts.iter().map(|s| (*s, inst.name.clone()))),
                    None => out.push((inst.site, inst.name.clone())),
                }
            }
        }
        out
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
        // A call's result is copied out of its frame (a method's into
        // the caller's arena, a stdlib leaf's built fresh); concatenation
        // is a fresh value.
        assert!(with(r#"let t = std::str::trim(self.f.s); self.put("n"); println(t);"#).is_empty());
        assert!(with(r#"let t = self.via_name(); self.put("n"); println(t);"#).is_empty());
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
    fn a_field_of_a_record_the_frame_holds_is_not_a_view() {
        let src = r#"
            type F { s: String = ""; }
            locus K { params { name: String = ""; } fn rename() { self.name = "x"; } }
            locus H {
                params { f: F = F { }; }
                fn put() { self.f = F { s: "n" }; }
                fn make() -> F { return F { s: "m" }; }
                fn record() { let r = self.make(); let s = r.s; self.put(); println(s); }
                fn handle() { let k = K { }; let n = k.name; k.rename(); println(n); }
            }
        "#;
        assert_eq!(flagged(src), ["n"]);
    }

    #[test]
    fn an_operand_held_across_a_later_operands_write_is_owned() {
        let src = r#"
            type E = enum { A(String), B };
            type R { a: String = ""; n: Int = 0; }
            fn take(s: String, n: Int) -> String { return s + "!"; }
            locus H {
                params { e: E = E::B; f: R = R { }; }
                fn reset() -> Int { self.e = E::B; self.f = R { }; return 0; }
                fn other() -> Int { return 1; }
                fn arg() -> String { return take(match self.e { E::A(s) -> s, E::B -> "", }, self.reset()); }
                fn binary() -> String { return self.f.a + to_string(self.reset()); }
                fn literal() -> R { return R { a: self.f.a, n: self.reset() }; }
                fn fine() -> String { return take(self.f.a, self.other()); }
            }
        "#;
        let got = owned_view_sites(&hale_syntax::parse_source(src).expect("parses"));
        assert_eq!(got.len(), 3, "{got:?}");
    }

    #[test]
    fn a_handle_is_known_by_its_resolved_type() {
        // Through an alias, through a binding's declared type, and by an
        // imported seed's minted name.
        let src = r#"
            type E = enum { A(String), B };
            locus Kid { params { e: E = E::B; } fn clear() { self.e = E::B; } }
            locus __lib_x_f_Ward { params { e: E = E::B; } fn clear() { self.e = E::B; } }
            type Child = Kid;
            fn child(k: Kid) -> Child { return k; }
            fn ward(w: x::Ward) -> x::Ward { return w; }
            fn opaque(k: Kid) -> Kid { return k; }
            locus H {
                params { k: Kid = Kid { }; w: x::Ward = x::Ward { }; }
                fn aliased() -> String { let c = child(self.k); let held = match c.e { E::A(s) -> s, E::B -> "", }; c.clear(); return held; }
                fn declared() -> String { let c: Kid = opaque(self.k); let held = match c.e { E::A(s) -> s, E::B -> "", }; c.clear(); return held; }
                fn imported() -> String { let c = ward(self.w); let held = match c.e { E::A(s) -> s, E::B -> "", }; c.clear(); return held; }
            }
        "#;
        let renames: BTreeMap<Vec<String>, String> =
            [(vec!["x".to_string(), "Ward".to_string()], "__lib_x_f_Ward".to_string())].into_iter().collect();
        let got = owned_view_sites_with(&hale_syntax::parse_source(src).expect("parses"), &renames);
        let lets = got.iter().filter(|(site, _)| matches!(site, Site::Node(_))).count();
        assert_eq!(lets, 3, "{got:?}");
    }

    #[test]
    fn a_callee_it_cannot_identify_writes_anything() {
        // An unknown free fn, an unknown method, a fn-valued field through
        // an alias and a field of a generic parameter's type: each may
        // write the field the binding views.
        let src = r#"
            type F { s: String = ""; }
            type Callback = fn(Int) -> Int;
            locus Hook<T> { params { cb: T; } }
            locus H {
                params { f: F = F { }; cb: Callback = nothing; k: Hook<Int> = Hook { cb: 0 }; }
                fn unknown_fn() -> String { let s = self.f.s; mystery(1); return s; }
                fn unknown_method() -> String { let s = self.f.s; self.k.vanish(); return s; }
                fn aliased_field() -> String { let s = self.f.s; self.cb(1); return s; }
                fn generic_field() -> String { let s = self.f.s; self.k.cb(1); return s; }
                fn known() -> String { let s = self.f.s; println("x"); return s; }
            }
        "#;
        let got = owned_view_sites(&hale_syntax::parse_source(src).expect("parses"));
        let lets = got.iter().filter(|(site, _)| matches!(site, Site::Node(_))).count();
        assert_eq!(lets, 4, "{got:?}");
    }

    /// Every question the pass asks of a type goes through `classify`:
    /// no `TypeExpr` variant is matched anywhere else, so a new spelling
    /// (an alias, an import, a minted name) cannot be classified one way
    /// in one place and another way in the next.
    #[test]
    fn types_are_classified_in_one_place() {
        let src = include_str!("view_bindings.rs");
        let body_start = src.find("    fn classify(&self, ty: &TypeExpr) -> Class {").expect("classify");
        let body_end = src.find("    /// A declared name's class").expect("classify_name");
        let needle = ["Type", "Expr::"].concat();
        for (i, _) in src.match_indices(&needle) {
            assert!(
                (body_start..body_end).contains(&i),
                "a type is matched outside `classify`, at byte {i}: {}",
                &src[i..src.len().min(i + 60)]
            );
        }
    }

    #[test]
    fn a_forms_read_is_a_view_of_its_cell() {
        // A form's synthesized `set` is the form's, not some other
        // locus's method named `set`; its read views the cell its later
        // write retires, and its arguments are copied in before it writes.
        let src = r#"
            type C { id: Int = 0; s: String = ""; }
            @form(hashmap)
            locus M { capacity { pool cells of C indexed_by id; } }
            locus Other { params { n: Int = 0; } fn set(c: C) { mystery(); } }
            fn rmw() {
                let m = M { };
                let e = m.get(1) or C { };
                m.set(C { id: 1, s: e.s });
            }
            fn held() {
                let m = M { };
                let e = m.get(1) or C { };
                m.set(C { id: 1, s: "x" });
                println(e.s);
            }
        "#;
        assert_eq!(flagged(src), ["e"]);
    }

    #[test]
    fn a_member_named_like_a_form_op_is_a_value_call() {
        // `pop` on a record member is no form's `pop`: only a receiver
        // known to hold a form takes a form's synthesized method.
        let src = r#"
            type F { s: String = ""; }
            type Hooks { pop: fn(Int) -> Int; }
            locus H {
                params { f: F = F { }; hooks: Hooks = Hooks { pop: one }; }
                fn hold() -> String { let s = self.f.s; self.hooks.pop(1); return s; }
            }
            fn one(x: Int) -> Int { return x; }
        "#;
        assert_eq!(flagged(src), ["s"]);
    }

    #[test]
    fn an_enum_constructor_holds_its_argument() {
        let src = r#"
            type E = enum { A(String), B };
            type F { s: String = ""; }
            locus H {
                params { f: F = F { }; }
                fn put() { self.f = F { s: "n" }; }
                fn body() { let e = E::A(self.f.s); self.put(); println(e); }
            }
        "#;
        assert_eq!(flagged(src), ["e"]);
    }

    #[test]
    fn a_publish_runs_the_handlers_writes() {
        let src = r#"
            type F { s: String = ""; }
            locus H {
                params { f: F = F { }; g: F = F { }; }
                bus { subscribe "t" as on_t of type String; }
                fn on_t(m: String) { self.f = F { s: m }; }
                fn held() { let s = self.f.s; "t" <- "x"; println(s); }
                fn other() { let s = self.g.s; "t" <- "x"; println(s); }
            }
        "#;
        assert_eq!(flagged(src), ["s"]);
    }

    #[test]
    fn a_fn_value_may_write_anything() {
        let src = r#"
            type F { s: String = ""; }
            locus H {
                params { g: F = F { }; }
                fn hold(cb: fn(Int) -> Int) { let s = self.g.s; cb(1); println(s); }
            }
        "#;
        assert_eq!(flagged(src), ["s"]);
        // A yield with no handler in the program runs nothing.
        assert!(with(r#"let s = self.g.s; yield; println(s);"#).is_empty());
    }
}
