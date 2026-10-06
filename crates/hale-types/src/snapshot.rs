//! The snapshot (F.40 phase 1.1b-ii): identity minted once, after
//! desugar, for every semantic site of the program.
//!
//! A site's identity is a `SiteId { seed, index }`
//! (`hale_graph::ids`). The index is one counter over the whole
//! merged program, so it is unique across seeds: F.39's owner table
//! keys rows by the bare index (`ExprId(u32)`), and a per-seed
//! numbering would collide there. The seed is the source unit the
//! site's span falls in (the bundle's `sources`, whose `base`/`len`
//! give each unit's range in the bundle-global space), or the
//! program's own ordinal when the caller has no source map. The
//! bundled stdlib is a seed of its own, named [`STDLIB_SEED`]: it is
//! parsed in its own coordinate space, which overlaps the source
//! map's first unit, so its spans cannot name its seed.
//!
//! Minting is idempotent: a site that already carries an id keeps
//! it, and the counter continues past the largest id present. That
//! is what lets the resolved-program step (`crate::resolved`) mint
//! the merged program after the bundle minted the user's: the user's
//! ids are kept, and the stdlib's sites and every node a later
//! desugar generated are numbered past them. Nothing numbers a site
//! after the mint; the F.39 pre-pass refuses one it finds unnumbered.
//!
//! What this pass records is one row per site: its id, its kind and
//! its span. Tables key on the id; witnesses render from the span
//! through the provenance store. Generated declarations (the JSON
//! parsers, the api surface) are minted like any other site, since
//! they exist in the program by the time the sequence has run; which
//! desugar generated a site is a second table, `origins` (phase
//! 1.1b-iii), read off the marker each desugar leaves.

use std::collections::BTreeMap;

use hale_graph::ids::{SeedId, SiteId};
use hale_syntax::ast::{
    ApiTransport, Block, BusMember, ClosureClause, ElseBranch, EpochSpec, Expr, IfStmt, KeyFilter,
    LValueSeg, LocusMember, MatchArmBody, MatchStmt, NodeId, OrDisposition, ParamInit, Pattern,
    PerspectiveMember, Program, RecoveryModifier, Stmt, TopDecl, TransportSpec, TypeDeclBody,
};
use hale_syntax::sites::{
    for_each_named_site, for_each_site, for_each_site_in_item, for_each_site_mut, SiteKind,
};
use hale_syntax::Span;

use crate::symbol::SourceFile;

/// Which desugar synthesized a site. `mint` records one for every site
/// it can tell was generated, by the marker the desugar leaves:
///
/// - `JsonParsers`: `json_gen`'s `__json_parse_<T>` / `__json_to_json_<T>`
///   fns (the name prefix) and its `JsonError` (`TypeDecl::synthetic`,
///   since a program may declare a `JsonError` of its own). The api
///   codecs' `JsonError` comes from the same generator and is recorded
///   here too.
/// - `ApiSurface`: `api_gen` parses everything it generates at
///   `API_SYNTH_BASE`, so any site whose span starts there; and the api
///   codecs, which `json_gen` parses at 0, by their `__api_decode_` /
///   `__api_encode_` prefix.
/// - `OmittedRun`: the `run` `desugar_omitted_run` adds, by its
///   `LifecycleDecl::synthesized` marker.
/// - `ChainDesugar`: the `let`s and assignments the chains rewrite
///   introduces bind `__hale_`-prefixed names, and the uses it writes
///   spell them. The calls it builds
///   (`.get(i)`, `.len()`) carry the chain's span and no marker, so
///   they have no row.
/// - `TopicDesugar`, `IntraLocusRewrite`, `ReprAccessors`: no row. Those
///   passes rewrite subjects and expressions in place and synthesize no
///   declaration; the calls the latter two build carry the rewritten
///   node's span and no marker. The topic and intra-locus rewrites run
///   only in the resolved-program step (`crate::resolved`), after every
///   entry point has minted; repr accessors run in the desugar sequence,
///   before the entry point's mint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    JsonParsers,
    ApiSurface,
    TopicDesugar,
    IntraLocusRewrite,
    ReprAccessors,
    OmittedRun,
    ChainDesugar,
}

/// One minted site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    pub id: SiteId,
    pub kind: SiteKind,
    pub span: Span,
}

/// The identities of one snapshot, in minting order.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// Every site, ordered by index.
    pub sites: Vec<Site>,
    /// The seeds, by `SeedId` index: a source unit's path, or the
    /// program's ordinal rendered as text when no source map was
    /// given.
    pub seeds: Vec<String>,
    /// The generated sites and the desugar that generated each,
    /// ordered by index. A site with no row was written.
    pub origins: Vec<(SiteId, Origin)>,
    /// Which declaration each use names (F.40 phase 2, use-site
    /// identity): a `Use` site, or an `Assign` site for the binding its
    /// head writes (whole, or through a field or an index), to the
    /// `Let`, `For` or `Binder` site it
    /// resolves to under lexical scoping (see [`resolve_uses`]). A use
    /// that names no local binding (a field, a top-level fn or const, a
    /// locus, a topic, nothing) has no row. Resolved once, by the mint.
    pub binding_of: BTreeMap<SiteId, SiteId>,
}

impl Snapshot {
    /// The site with this id, if the snapshot minted it.
    pub fn site(&self, id: SiteId) -> Option<&Site> {
        self.sites
            .binary_search_by_key(&id.index, |s| s.id.index)
            .ok()
            .map(|i| &self.sites[i])
            .filter(|s| s.id == id)
    }

    /// The full identity of the site an AST node carries: its seed
    /// joined to the index the node holds. `None` for a `NONE` id and
    /// for an index this snapshot did not mint. The index alone is
    /// unique (one counter numbers every seed), so this is one binary
    /// search.
    pub fn site_id(&self, node: NodeId) -> Option<SiteId> {
        if node.is_none() {
            return None;
        }
        self.sites
            .binary_search_by_key(&node.0, |s| s.id.index)
            .ok()
            .map(|i| self.sites[i].id)
    }

    /// The declaration the use at `node` names — an identifier
    /// expression, or an assignment's head by its statement's id — if it
    /// names a local binding this snapshot minted.
    pub fn declaration_of(&self, node: NodeId) -> Option<SiteId> {
        self.binding_of.get(&self.site_id(node)?).copied()
    }

    /// The desugar that generated this site, if one did.
    pub fn origin(&self, id: SiteId) -> Option<Origin> {
        self.origins
            .binary_search_by_key(&id.index, |(s, _)| s.index)
            .ok()
            .map(|i| self.origins[i])
            .filter(|(s, _)| *s == id)
            .map(|(_, o)| o)
    }

    pub fn len(&self) -> usize {
        self.sites.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }
}

/// The name of the bundled stdlib's seed. A program minted under this
/// name is its own seed whatever the source map says (see the module
/// docs), recorded after the source map's units.
pub const STDLIB_SEED: &str = "<stdlib>";

/// The seed of a span: the source unit whose bundle-global range holds
/// its start, if the source map has one.
fn seed_of(sources: &[SourceFile], span: Span) -> Option<SeedId> {
    sources
        .iter()
        .position(|u| span.start.0 >= u.base && span.start.0 < u.base.saturating_add(u.len))
        .map(|i| SeedId(i as u32))
}

/// Number every site of `programs` that has none, and nothing else: the
/// numbering [`mint`] does, for a caller that needs the ids on the tree
/// before a rewrite and mints the result later (the resolved-program
/// step). The counter continues past the largest id already present, so
/// numbering twice, or numbering a merged program whose user half the
/// bundle already minted, keeps every existing id.
pub fn number<'a>(programs: impl IntoIterator<Item = &'a mut Program>) {
    let mut programs: Vec<&mut Program> = programs.into_iter().collect();
    let mut next: u32 = 0;
    for p in programs.iter() {
        for_each_site(p, &mut |_, _, id| {
            if !id.is_none() {
                next = next.max(id.0 + 1);
            }
        });
    }
    for p in programs.iter_mut() {
        for_each_site_mut(p, &mut |_, _, id| {
            if id.is_none() {
                *id = NodeId(next);
                next += 1;
            }
        });
    }
}

/// Mint an identity for every site of `programs` that has none, and
/// return every site's row, every generated site's origin and every
/// use's declaration ([`Snapshot::binding_of`]). `sources` is the
/// bundle's source map (the seed of a site is the unit its span falls
/// in); when it is empty, each program is its own seed, in the order
/// given. A program named [`STDLIB_SEED`] is its own seed either way.
pub fn mint<'a>(
    programs: impl IntoIterator<Item = (&'a str, &'a mut Program)>,
    sources: &[SourceFile],
) -> Snapshot {
    let mut programs: Vec<(&str, &mut Program)> = programs.into_iter().collect();
    number(programs.iter_mut().map(|(_, p)| &mut **p));
    let mut seeds: Vec<String> = sources.iter().map(|u| u.path.clone()).collect();
    let mut sites = Vec::new();
    for (path, p) in programs.iter() {
        let own_seed = if sources.is_empty() || *path == STDLIB_SEED {
            seeds.push(path.to_string());
            Some(SeedId(seeds.len() as u32 - 1))
        } else {
            None
        };
        for_each_site(p, &mut |kind, span, id| {
            let seed = own_seed
                .or_else(|| seed_of(sources, span))
                .unwrap_or(SeedId(0));
            sites.push(Site { id: SiteId::new(seed, id.0), kind, span });
        });
    }
    sites.sort_by_key(|s| s.id.index);
    // One id, one site: a second site carrying an id is a walk that
    // copied a node without minting it, and the rows keyed by that id
    // would answer for both.
    if let Some(w) = sites.windows(2).find(|w| w[0].id.index == w[1].id.index) {
        panic!(
            "two sites share snapshot id {}: {:?} at {:?} and {:?} at {:?}",
            w[0].id.index, w[0].kind, w[0].span, w[1].kind, w[1].span
        );
    }
    let mut snapshot = Snapshot { sites, seeds, origins: Vec::new(), binding_of: BTreeMap::new() };

    let mut by_index: BTreeMap<u32, Origin> = BTreeMap::new();
    for (_, p) in programs.iter() {
        origins_of(p, &mut by_index);
    }
    snapshot.origins = by_index
        .into_iter()
        .filter_map(|(index, origin)| {
            let i = snapshot
                .sites
                .binary_search_by_key(&index, |s| s.id.index)
                .ok()?;
            Some((snapshot.sites[i].id, origin))
        })
        .collect();

    let mut uses: BTreeMap<u32, u32> = BTreeMap::new();
    for (_, p) in programs.iter() {
        resolve_uses(p, &mut uses);
    }
    snapshot.binding_of = uses
        .into_iter()
        .filter_map(|(u, d)| Some((snapshot.site_id(NodeId(u))?, snapshot.site_id(NodeId(d))?)))
        .collect();
    RESOLUTIONS.with(|n| n.set(n.get() + 1));
    snapshot
}

thread_local! {
    static RESOLUTIONS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many times this thread resolved a snapshot's uses: the
/// accounting a test reads to pin that each snapshot resolves them
/// once, in its mint, and no reader resolves them again.
pub fn resolutions_on_this_thread() -> u64 {
    RESOLUTIONS.with(|n| n.get())
}

/// Which declaration each use of `p` names, by index: an identifier
/// expression's id (or a bare assignment's statement id) to the id of
/// the `let`, `for` or binder it resolves to.
///
/// The rule is the checker's, `check::ScopeStack`: a body's parameters
/// are one frame and every block another; a `let` binds from the
/// statement after it (its value is read first, so `let x = x + 1;`
/// reads the outer `x`) and a later `let` of the name in the same frame
/// shadows it; a `for` variable is a frame around its body; a match arm
/// is a frame holding its pattern's bindings, over its guard and body; a
/// `shm_write` binding is a frame over its body; and the substitute or
/// `fail` payload of an `or` sees an implicit `err`, a frame of its own.
/// Lookup is innermost first. A name no frame holds is not a local: no
/// row. The implicit `err` has no site, so it shadows and resolves to
/// nothing, as does any binding whose declaration was not minted.
fn resolve_uses(p: &Program, out: &mut BTreeMap<u32, u32>) {
    let mut w = UseScopes { frames: Vec::new(), out };
    w.items(&p.items);
}

struct UseScopes<'p, 'o> {
    frames: Vec<Vec<(&'p str, NodeId)>>,
    out: &'o mut BTreeMap<u32, u32>,
}

impl<'p> UseScopes<'p, '_> {
    fn bind(&mut self, name: &'p str, decl: NodeId) {
        self.frames.last_mut().expect("a frame").push((name, decl));
    }

    /// Record what `name`, spelled at the site `at`, resolves to.
    fn resolve(&mut self, name: &str, at: NodeId) {
        if at.is_none() {
            return;
        }
        let decl = self
            .frames
            .iter()
            .rev()
            .flat_map(|f| f.iter().rev())
            .find(|(n, _)| *n == name)
            .map(|(_, d)| *d);
        if let Some(d) = decl.filter(|d| !d.is_none()) {
            self.out.insert(at.0, d.0);
        }
    }

    /// An expression outside every body: nothing is in scope but what it
    /// binds itself.
    fn detached(&mut self, e: &'p Expr) {
        self.frames.push(Vec::new());
        self.expr(e);
        self.frames.pop();
    }

    fn items(&mut self, items: &'p [TopDecl]) {
        for item in items {
            match item {
                TopDecl::Fn(fd) => self.fn_decl(fd),
                TopDecl::Locus(l) => {
                    if let Some(form) = &l.form {
                        for a in &form.args {
                            self.detached(&a.value);
                        }
                    }
                    for m in &l.members {
                        self.locus_member(m);
                    }
                }
                TopDecl::Perspective(pd) => {
                    for m in &pd.members {
                        match m {
                            PerspectiveMember::Params(pb) => self.params_block(pb),
                            PerspectiveMember::StableWhen(b) => {
                                self.frames.push(Vec::new());
                                self.block(b);
                                self.frames.pop();
                            }
                            PerspectiveMember::Fn(fd) => self.fn_decl(fd),
                            PerspectiveMember::SerializeAs(_) | PerspectiveMember::Bus(_) => {}
                        }
                    }
                }
                TopDecl::Module(md) => self.items(&md.items),
                TopDecl::Const(c) => self.detached(&c.value),
                TopDecl::Type(t) => self.type_decl(t),
                TopDecl::Interface(_)
                | TopDecl::Topic(_)
                | TopDecl::Group(_)
                | TopDecl::RingLayout(_)
                | TopDecl::Target(_)
                | TopDecl::Role(_)
                | TopDecl::Claims(_)
                | TopDecl::Constitution(_)
                | TopDecl::Unit(_) => {}
            }
        }
    }

    fn type_decl(&mut self, t: &'p hale_syntax::ast::TypeDecl) {
        match &t.body {
            TypeDeclBody::Struct(fields) => {
                for f in fields {
                    if let Some(d) = &f.default {
                        self.detached(d);
                    }
                }
            }
            // GH #1076: a range's bounds are expressions.
            TypeDeclBody::Scalar(s) => {
                for c in &s.clauses {
                    if let hale_syntax::ast::ScalarClause::Range { lo, hi, .. } = c {
                        self.detached(lo);
                        self.detached(hi);
                    }
                }
            }
            TypeDeclBody::Alias(_) | TypeDeclBody::Enum(_) => {}
        }
    }

    fn params_block(&mut self, pb: &'p hale_syntax::ast::ParamsBlock) {
        for p in &pb.params {
            if let ParamInit::Value(e) = &p.init {
                self.detached(e);
            }
        }
    }

    fn locus_member(&mut self, m: &'p LocusMember) {
        match m {
            LocusMember::Fn(fd) => self.fn_decl(fd),
            LocusMember::Lifecycle(ld) => self.body(&ld.params, &ld.body),
            LocusMember::Mode(md) => self.body(&md.params, &md.body),
            LocusMember::Failure(fd) => self.body(&fd.params, &fd.body),
            LocusMember::Params(pb) => self.params_block(pb),
            LocusMember::Const(c) => self.detached(&c.value),
            LocusMember::Type(t) => self.type_decl(t),
            LocusMember::BirthCheck(bc) => {
                self.detached(&bc.cond);
                if let Some(p) = &bc.payload {
                    self.detached(p);
                }
            }
            LocusMember::Closure(cd) => {
                if let Some(a) = &cd.assertion {
                    self.detached(&a.left);
                    self.detached(&a.right);
                    self.detached(&a.tolerance);
                }
                for c in &cd.clauses {
                    if let ClosureClause::Epoch(EpochSpec::Duration(e)) = c {
                        self.detached(e);
                    }
                }
            }
            LocusMember::Bus(bb) => {
                for bm in &bb.members {
                    if let BusMember::Subscribe {
                        key_filter: Some(KeyFilter::Specific { expr, .. }),
                        ..
                    } = bm
                    {
                        self.detached(expr);
                    }
                }
            }
            LocusMember::Bindings(bb) => {
                for entry in &bb.entries {
                    if let TransportSpec::Adapter { inits, .. } = &entry.transport {
                        for i in inits {
                            self.detached(&i.value);
                        }
                    }
                    if let Some(codec) = &entry.codec {
                        for i in &codec.inits {
                            self.detached(&i.value);
                        }
                    }
                }
                if let Some(api) = &bb.api {
                    let ApiTransport::Unix { path, .. } = &api.transport;
                    self.detached(path);
                    if let Some(r) = &api.roles {
                        self.detached(&r.expr);
                    }
                    if let Some(h) = &api.http {
                        self.detached(&h.host);
                        self.detached(&h.port);
                        if let Some(p) = &h.principals {
                            self.detached(p);
                        }
                    }
                }
            }
            LocusMember::Contract(_)
            | LocusMember::Capacity(_)
            | LocusMember::Placement(_)
            | LocusMember::Topology(_)
            | LocusMember::Claims(_) => {}
        }
    }

    fn fn_decl(&mut self, fd: &'p hale_syntax::ast::FnDecl) {
        self.body(&fd.params, &fd.body);
    }

    /// A fn's or a hook's body: its parameters are a frame (the
    /// checker's `check_fn`), the body block another. A parameter's
    /// default is read where no parameter is in scope.
    fn body(&mut self, params: &'p [hale_syntax::ast::Param], body: &'p Block) {
        for p in params {
            if let Some(d) = &p.default {
                self.detached(d);
            }
        }
        self.frames.push(Vec::new());
        for p in params {
            self.bind(&p.name.name, p.name.id);
        }
        self.block(body);
        self.frames.pop();
    }

    fn block(&mut self, b: &'p Block) {
        self.frames.push(Vec::new());
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(t) = &b.tail {
            self.expr(t);
        }
        self.frames.pop();
    }

    fn if_chain(&mut self, i: &'p IfStmt) {
        self.expr(&i.cond);
        self.block(&i.then_block);
        match i.else_block.as_deref() {
            Some(ElseBranch::Else(b)) => self.block(b),
            Some(ElseBranch::ElseIf(n)) => self.if_chain(n),
            None => {}
        }
    }

    fn match_arms(&mut self, m: &'p MatchStmt) {
        self.expr(&m.scrutinee);
        for arm in &m.arms {
            self.frames.push(Vec::new());
            self.pattern(&arm.pattern);
            if let Some(g) = &arm.guard {
                self.expr(g);
            }
            match &arm.body {
                MatchArmBody::Expr(e) => self.expr(e),
                MatchArmBody::Block(b) => self.block(b),
            }
            self.frames.pop();
        }
    }

    fn pattern(&mut self, p: &'p Pattern) {
        match p {
            Pattern::Binding(i) => self.bind(&i.name, i.id),
            Pattern::Constructor { args, .. } | Pattern::Tuple(args, _) => {
                for a in args {
                    self.pattern(a);
                }
            }
            Pattern::Literal(..) | Pattern::Wildcard(_) => {}
        }
    }

    /// An `or`'s disposition: the substitute and the `fail` payload see
    /// the implicit `err`.
    fn disposition(&mut self, d: &'p OrDisposition) {
        match d {
            OrDisposition::Substitute(e) | OrDisposition::Fail(e, _) => {
                self.frames.push(vec![("err", NodeId::NONE)]);
                self.expr(e);
                self.frames.pop();
            }
            OrDisposition::Raise(_) | OrDisposition::Discard(_) | OrDisposition::Wait(_) => {}
        }
    }

    fn stmt(&mut self, s: &'p Stmt) {
        match s {
            Stmt::Let { name, value, id, .. } => {
                self.expr(value);
                self.bind(&name.name, *id);
            }
            Stmt::LetTuple { names, value, .. } => {
                self.expr(value);
                for n in names {
                    self.bind(&n.name, n.id);
                }
            }
            Stmt::Assign { target, value, id, .. } => {
                self.expr(value);
                for seg in &target.tail {
                    if let LValueSeg::Index(ix) = seg {
                        self.expr(ix);
                    }
                }
                self.resolve(&target.head.name, *id);
            }
            Stmt::If(i) => self.if_chain(i),
            Stmt::Match(m) => self.match_arms(m),
            Stmt::For { name, iter, body, id, .. } => {
                self.expr(iter);
                self.frames.push(Vec::new());
                self.bind(&name.name, *id);
                self.block(body);
                self.frames.pop();
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond);
                self.block(body);
            }
            Stmt::Return(e, _) => {
                if let Some(e) = e {
                    self.expr(e);
                }
            }
            Stmt::Fail { value, .. } => self.expr(value),
            Stmt::Block(b) => self.block(b),
            Stmt::Recovery { args, modifier, .. } => {
                for a in args {
                    self.expr(a);
                }
                match modifier {
                    Some(RecoveryModifier::For(e) | RecoveryModifier::Until(e)) => self.expr(e),
                    None => {}
                }
            }
            Stmt::Violate { payload, .. } => {
                if let Some(p) = payload {
                    self.expr(p);
                }
            }
            Stmt::Send { subject, value, or_disposition, .. } => {
                self.expr(subject);
                self.expr(value);
                if let Some(d) = or_disposition {
                    self.disposition(d);
                }
            }
            // The checker binds the view and checks the body's
            // statements in that one frame.
            Stmt::ShmWrite { max, binding, body, .. } => {
                self.expr(max);
                self.frames.push(Vec::new());
                self.bind(&binding.name, binding.id);
                for s in &body.stmts {
                    self.stmt(s);
                }
                if let Some(t) = &body.tail {
                    self.expr(t);
                }
                self.frames.pop();
            }
            Stmt::Expr(e) => self.expr(e),
            Stmt::Break(_)
            | Stmt::Continue(_)
            | Stmt::Yield(_)
            | Stmt::Terminate(_)
            | Stmt::Reperspective { .. } => {}
        }
    }

    fn expr(&mut self, e: &'p Expr) {
        match e {
            Expr::Ident(i) => self.resolve(&i.name, i.id),
            Expr::Literal(..) | Expr::Path(_) | Expr::KwSelf(_) => {}
            Expr::Binary { left, right, .. } => {
                self.expr(left);
                self.expr(right);
            }
            Expr::Unary { operand, .. } => self.expr(operand),
            Expr::Call { callee, args, .. } => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
            }
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
            Expr::Struct { inits, .. } => {
                for i in inits {
                    self.expr(&i.value);
                }
            }
            Expr::Block(b) => self.block(b),
            Expr::If(i) => self.if_chain(i),
            Expr::Match(m) => self.match_arms(m),
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
            Expr::Or { inner, disposition, .. } => {
                self.expr(inner);
                self.disposition(disposition);
            }
        }
    }
}

/// Record the origin of every generated site of `p`, by index. A
/// declaration's origin covers every site inside it and is recorded
/// first; the per-site markers only fill what that left.
fn origins_of(p: &Program, out: &mut BTreeMap<u32, Origin>) {
    fn items(decls: &[TopDecl], out: &mut BTreeMap<u32, Origin>) {
        for item in decls {
            let origin = match item {
                TopDecl::Fn(fd)
                    if fd.name.name.starts_with("__json_parse_")
                        || fd.name.name.starts_with("__json_to_json_") =>
                {
                    Some(Origin::JsonParsers)
                }
                TopDecl::Fn(fd)
                    if fd.name.name.starts_with("__api_decode_")
                        || fd.name.name.starts_with("__api_encode_") =>
                {
                    Some(Origin::ApiSurface)
                }
                TopDecl::Type(t) if t.synthetic && t.name.name == "JsonError" => {
                    Some(Origin::JsonParsers)
                }
                TopDecl::Module(md) => {
                    items(&md.items, out);
                    None
                }
                _ => None,
            };
            if let Some(origin) = origin {
                for_each_site_in_item(item, &mut |_, _, id| {
                    out.entry(id.0).or_insert(origin);
                });
            }
        }
    }
    items(&p.items, out);

    // The hooks `desugar_omitted_run` added, by the marker it leaves.
    fn synthesized_hooks(decls: &[TopDecl], out: &mut std::collections::BTreeSet<u32>) {
        for item in decls {
            match item {
                TopDecl::Locus(l) => {
                    for m in &l.members {
                        if let hale_syntax::ast::LocusMember::Lifecycle(lc) = m {
                            if lc.synthesized {
                                out.insert(lc.id.0);
                            }
                        }
                    }
                }
                TopDecl::Module(md) => synthesized_hooks(&md.items, out),
                _ => {}
            }
        }
    }
    let mut synthesized = std::collections::BTreeSet::new();
    synthesized_hooks(&p.items, &mut synthesized);
    for_each_named_site(p, &mut |kind, span, name, id| {
        let origin = if span.start.0 >= hale_syntax::api_gen::API_SYNTH_BASE {
            Some(Origin::ApiSurface)
        } else {
            match kind {
                SiteKind::Lifecycle if synthesized.contains(&id.0) => Some(Origin::OmittedRun),
                SiteKind::Let | SiteKind::Assign | SiteKind::For | SiteKind::Use
                    if name.is_some_and(|n| n.starts_with("__hale_")) =>
                {
                    Some(Origin::ChainDesugar)
                }
                _ => None,
            }
        };
        if let Some(origin) = origin {
            out.entry(id.0).or_insert(origin);
        }
    });
}
