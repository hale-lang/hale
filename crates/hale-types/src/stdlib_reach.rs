//! Which of the stdlib's analysis copy a program can reach.
//!
//! The check's analyses read the stdlib's analysis copy
//! ([`crate::stdlib_bodies`]) beside the program, so that a call into
//! the stdlib has a body to walk. A body the program cannot reach is
//! never a callee of the program's, so the check walks only the items it
//! can reach: the checker's walk of the copy, the owned-view pass over
//! it and the allocation summary's rows. A program pays for the stdlib
//! it uses, not for all of it.
//!
//! The reach is a set of the copy's top-level items, closed under what a
//! reached item (the program's own first) names:
//!
//! - a declaration's own name: a fn, a locus, a type, an interface, a
//!   perspective, a topic, a constant, a unit. A call into the stdlib is
//!   to a fn by its name, or to a method of a value whose type some
//!   reached item names (a parameter's, a field's, a return's, a
//!   literal's), so the locus declaring the method is reached through
//!   its type;
//! - a `std::` path, through the stdlib's renames to the mangled name its
//!   declaration carries;
//! - a member of a reached stdlib handle's namespace: a method call on
//!   the handle is sugar for the namespace's fn (`f.write_line(x)` is
//!   `std::io::file::write_line(f, x)`), so every name a reached item
//!   spells is tried as one;
//! - a method named like one of an interface's or a perspective's, of
//!   one reached or declared by the program: a call through either fans
//!   out to every locus declaring a method of the name.
//!
//! Reaching a locus reaches every member, hook, mode and handler it
//! declares, and the items they name. Two sets are reached whatever the
//! program names, because the analyses resolve into them with no name at
//! the call: every method named like a bus handler, the program's or the
//! stdlib's (a publish, a `yield` or a park may run any of them), and
//! every stdlib fn the stdlib reads as a value (an indirect call may
//! reach any of them), with every item that reads a method as a value.
//!
//! These rules are what the closure starts from, not what it rests on:
//! each analysis that reads the restricted copy checks that what it
//! resolved stays inside the reach, and widens the reach and runs again
//! when it does not ([`StdlibReach::widen`]).

use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

use hale_syntax::ast::{BusMember, LocusMember, PerspectiveMember, Program, TopDecl};
use hale_syntax::names::{for_each_spelled_in_item, Spelled};

/// A set of the stdlib analysis copy's top-level items, by their index in
/// [`crate::stdlib_bodies::program`]'s items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdlibReach {
    items: Vec<bool>,
}

impl StdlibReach {
    /// Every item of the copy.
    pub fn whole() -> StdlibReach {
        StdlibReach { items: vec![true; index().refs.len()] }
    }

    /// Whether `item` (an index into the copy's items) is reached.
    pub fn contains(&self, item: usize) -> bool {
        self.items.get(item).copied().unwrap_or(false)
    }

    /// Whether every item of the copy is reached.
    pub fn is_whole(&self) -> bool {
        self.items.iter().all(|r| *r)
    }

    /// How many items are reached.
    pub fn count(&self) -> usize {
        self.items.iter().filter(|r| **r).count()
    }

    /// The reached items, ascending.
    pub fn items(&self) -> impl Iterator<Item = usize> + '_ {
        self.items.iter().enumerate().filter(|(_, r)| **r).map(|(i, _)| i)
    }

    /// The reach of `programs`: the checked programs after the desugar
    /// sequence, with what the api expansion and the TCP substrate
    /// appended to them.
    pub fn of(programs: &[&Program]) -> StdlibReach {
        let index = index();
        let mut reach = StdlibReach { items: vec![false; index.refs.len()] };
        let mut closure = Closure::new(index, &mut reach);
        for &i in &index.always {
            closure.add(i);
        }
        for p in programs {
            for item in hale_syntax::ast::flat_decls(&p.items) {
                // The program's own bus handlers are delivered to as the
                // stdlib's are, and its interfaces and perspectives
                // dispatch to every method of their methods' names.
                let mut members: Vec<&str> = Vec::new();
                match item {
                    TopDecl::Locus(l) => members.extend(handler_names(l)),
                    TopDecl::Interface(i) => members.extend(i.methods.iter().map(|m| m.name.name.as_str())),
                    TopDecl::Perspective(p) => members.extend(perspective_fns(p)),
                    _ => {}
                }
                for m in members {
                    for &i in index.methods.get(m).into_iter().flatten() {
                        closure.add(i);
                    }
                }
            }
            for item in &p.items {
                for_each_spelled_in_item(item, &mut |s| closure.spell(s));
            }
        }
        closure.run();
        reach
    }

    /// Add `items` and close the reach again: what an analysis resolved
    /// beyond it. Returns whether the reach grew.
    pub fn widen(&mut self, items: impl IntoIterator<Item = usize>) -> bool {
        let before = self.count();
        let index = index();
        let reached: Vec<usize> = self.items().collect();
        let mut closure = Closure::new(index, self);
        // What the reached items spell and which handles they are, so an
        // item reached now meets them as it would have in the first
        // closure. The program's own spellings are not kept: they reached
        // what they name the first time, and a handle reached now is
        // tried against the stdlib's.
        for i in reached {
            for &n in &index.names[i] {
                closure.name(n);
            }
            if let Some(ns) = &index.handle_ns[i] {
                closure.handles.insert(ns.clone());
            }
        }
        for i in items {
            closure.add(i);
        }
        closure.run();
        self.count() > before
    }
}

/// The copy's items and what they declare and name, once per process.
struct Index {
    /// A declaration's name → the items declaring it (a module's
    /// declarations are its item's).
    declares: HashMap<&'static str, Vec<usize>>,
    /// A member fn's name → the items declaring one (a locus's or a
    /// perspective's).
    methods: HashMap<&'static str, Vec<usize>>,
    /// A public path → the mangled name it is the spelling of.
    renames: HashMap<&'static [&'static str], &'static str>,
    /// Per item, the items it names: a declaration's name, a path, the
    /// methods of its interfaces' and perspectives' method names.
    refs: Vec<Vec<usize>>,
    /// Per item, the names it spells (for the handle rule).
    names: Vec<Vec<&'static str>>,
    /// Per item that is a locus with a public path, its namespace: the
    /// path without its last segment, whose fns its methods are sugar
    /// for.
    handle_ns: Vec<Option<Vec<&'static str>>>,
    /// A last segment → each namespace with a public fn of that name, and
    /// the items declaring the fn's mangled name.
    ns_members: HashMap<&'static str, Vec<(Vec<&'static str>, Vec<usize>)>>,
    /// The items reached whatever the program names.
    always: Vec<usize>,
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let empty: &'static [TopDecl] = &[];
        let items: &'static [TopDecl] = crate::stdlib_bodies::program().map_or(empty, |p| p.items.as_slice());
        let mut declares: HashMap<&'static str, Vec<usize>> = HashMap::new();
        let mut methods: HashMap<&'static str, Vec<usize>> = HashMap::new();
        let mut handlers: BTreeSet<&'static str> = BTreeSet::new();
        let note = |at: &mut HashMap<&'static str, Vec<usize>>, n: &'static str, i: usize| {
            let row = at.entry(n).or_default();
            if row.last() != Some(&i) {
                row.push(i);
            }
        };
        for (i, item) in items.iter().enumerate() {
            for d in hale_syntax::ast::flat_decls(std::slice::from_ref(item)) {
                if let Some(n) = declared_name(d) {
                    note(&mut declares, n, i);
                }
                match d {
                    TopDecl::Locus(l) => {
                        handlers.extend(handler_names(l));
                        for m in &l.members {
                            if let LocusMember::Fn(f) = m {
                                note(&mut methods, &f.name.name, i);
                            }
                        }
                    }
                    TopDecl::Perspective(p) => {
                        for f in perspective_fns(p) {
                            note(&mut methods, f, i);
                        }
                    }
                    _ => {}
                }
            }
        }
        let renames: HashMap<&'static [&'static str], &'static str> =
            hale_stdlib::PATH_RENAMES.iter().map(|(path, mangled)| (*path, *mangled)).collect();
        let public: HashMap<&'static str, &'static [&'static str]> =
            hale_stdlib::PATH_RENAMES.iter().map(|(path, mangled)| (*mangled, *path)).collect();
        let mut refs = Vec::with_capacity(items.len());
        let mut names = Vec::with_capacity(items.len());
        let mut handle_ns = Vec::with_capacity(items.len());
        for item in items {
            let mut to: BTreeSet<usize> = BTreeSet::new();
            let mut spelled: BTreeSet<&'static str> = BTreeSet::new();
            for_each_spelled_in_item(item, &mut |s| match s {
                Spelled::Name(n) => {
                    to.extend(declares.get(n).into_iter().flatten());
                    spelled.insert(n);
                }
                Spelled::Text(_) => {}
                Spelled::Path(segs) => {
                    for m in path_renames(segs, &renames) {
                        to.extend(declares.get(m).into_iter().flatten());
                    }
                }
            });
            for d in hale_syntax::ast::flat_decls(std::slice::from_ref(item)) {
                let dispatched: Vec<&str> = match d {
                    TopDecl::Interface(it) => it.methods.iter().map(|m| m.name.name.as_str()).collect(),
                    TopDecl::Perspective(p) => perspective_fns(p).collect(),
                    _ => Vec::new(),
                };
                for m in dispatched {
                    to.extend(methods.get(m).into_iter().flatten());
                }
            }
            refs.push(to.into_iter().collect());
            names.push(spelled.into_iter().collect());
            handle_ns.push(match item {
                TopDecl::Locus(l) => {
                    public.get(l.name.name.as_str()).and_then(|p| p.split_last()).map(|(_, ns)| ns.to_vec())
                }
                _ => None,
            });
        }
        let mut ns_members: HashMap<&'static str, Vec<(Vec<&'static str>, Vec<usize>)>> = HashMap::new();
        for (path, mangled) in hale_stdlib::PATH_RENAMES {
            let Some((last, ns)) = path.split_last() else { continue };
            if let Some(at) = declares.get(mangled) {
                ns_members.entry(last).or_default().push((ns.to_vec(), at.clone()));
            }
        }
        let mut always: BTreeSet<usize> = BTreeSet::new();
        for h in &handlers {
            always.extend(methods.get(h).into_iter().flatten());
        }
        for item in items {
            let mut values = Vec::new();
            crate::fn_values::value_names(std::slice::from_ref(item), &mut values);
            for v in values {
                match &v {
                    crate::fn_values::ValueName::Ident(n) => always.extend(declares.get(n.as_str()).into_iter().flatten()),
                    crate::fn_values::ValueName::Path(p) => {
                        always.extend(declares.get(p.rsplit("::").next().unwrap_or(p)).into_iter().flatten())
                    }
                    // A method read as a value is no fn's value; the copy
                    // reads none (`the_copy_reads_no_method_as_a_value`).
                    crate::fn_values::ValueName::Member(_) => {}
                }
            }
        }
        Index { declares, methods, renames, refs, names, handle_ns, ns_members, always: always.into_iter().collect() }
    })
}

/// The mangled name each prefix of a path is the public spelling of.
fn path_renames(
    segs: &[hale_syntax::ast::Ident],
    renames: &HashMap<&'static [&'static str], &'static str>,
) -> Vec<&'static str> {
    let words: Vec<&str> = segs.iter().map(|s| s.name.as_str()).collect();
    (1..=words.len()).filter_map(|k| renames.get(&words[..k]).copied()).collect()
}

/// The name a declaration declares, if it is one a call or a type names.
fn declared_name(d: &TopDecl) -> Option<&str> {
    Some(match d {
        TopDecl::Fn(x) => &x.name.name,
        TopDecl::Locus(x) => &x.name.name,
        TopDecl::Perspective(x) => &x.name.name,
        TopDecl::Interface(x) => &x.name.name,
        TopDecl::Type(x) => &x.name.name,
        TopDecl::Topic(x) => &x.name.name,
        TopDecl::Const(x) => &x.name.name,
        TopDecl::Unit(x) => &x.name.name,
        _ => return None,
    })
}

/// A perspective's member fns, by name.
fn perspective_fns(p: &hale_syntax::ast::PerspectiveDecl) -> impl Iterator<Item = &str> {
    p.members.iter().filter_map(|m| match m {
        PerspectiveMember::Fn(f) => Some(f.name.name.as_str()),
        _ => None,
    })
}

/// A locus's bus handlers, by name.
fn handler_names(l: &hale_syntax::ast::LocusDecl) -> impl Iterator<Item = &str> {
    l.members
        .iter()
        .filter_map(|m| match m {
            LocusMember::Bus(b) => Some(&b.members),
            _ => None,
        })
        .flatten()
        .filter_map(|bm| match bm {
            BusMember::Subscribe { handler, .. } => Some(handler.name.as_str()),
            _ => None,
        })
}

/// The closure's working state.
struct Closure<'i, 'r> {
    index: &'i Index,
    reach: &'r mut StdlibReach,
    /// Every name a reached item spells, the program's own included, that
    /// is the last segment of some namespace's public fn.
    spelled: BTreeSet<&'static str>,
    /// The handle namespaces of the reached loci.
    handles: BTreeSet<Vec<&'static str>>,
    work: Vec<usize>,
}

impl<'i, 'r> Closure<'i, 'r> {
    fn new(index: &'i Index, reach: &'r mut StdlibReach) -> Closure<'i, 'r> {
        Closure { index, reach, spelled: BTreeSet::new(), handles: BTreeSet::new(), work: Vec::new() }
    }

    fn add(&mut self, i: usize) {
        if let Some(r) = self.reach.items.get_mut(i) {
            if !*r {
                *r = true;
                self.work.push(i);
            }
        }
    }

    /// Something a program spells.
    fn spell(&mut self, s: Spelled<'_>) {
        let index = self.index;
        match s {
            Spelled::Name(n) => {
                for &i in index.declares.get(n).into_iter().flatten() {
                    self.add(i);
                }
                self.name(n);
            }
            Spelled::Text(_) => {}
            // Every public path is a `std::` one.
            Spelled::Path(segs) if segs.first().is_some_and(|s| s.name == "std") => {
                for m in path_renames(segs, &index.renames) {
                    for &i in index.declares.get(m).into_iter().flatten() {
                        self.add(i);
                    }
                }
            }
            Spelled::Path(_) => {}
        }
    }

    /// A name spelled: a reached handle's member of that name is its
    /// namespace's fn.
    fn name(&mut self, n: &str) {
        let index = self.index;
        let Some((&n, members)) = index.ns_members.get_key_value(n) else { return };
        if !self.spelled.insert(n) {
            return;
        }
        for (ns, at) in members {
            if self.handles.contains(ns) {
                for &i in at {
                    self.add(i);
                }
            }
        }
    }

    fn run(&mut self) {
        let index = self.index;
        while let Some(i) = self.work.pop() {
            for &j in &index.refs[i] {
                self.add(j);
            }
            for &n in &index.names[i] {
                self.name(n);
            }
            if let Some(ns) = &index.handle_ns[i] {
                if self.handles.insert(ns.clone()) {
                    for (last, members) in &index.ns_members {
                        if !self.spelled.contains(*last) {
                            continue;
                        }
                        for (at_ns, at) in members {
                            if at_ns == ns {
                                for &k in at {
                                    self.add(k);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The closure reads only `std::` paths through the renames: every
    /// public path is one.
    #[test]
    fn every_public_path_is_a_std_path() {
        for (path, mangled) in hale_stdlib::PATH_RENAMES {
            assert_eq!(path.first(), Some(&"std"), "`{mangled}`'s public path");
        }
    }

    /// The copy declares no perspective: a perspective's fns are no rows
    /// of the allocation summary, so the summary's reach reads loci and
    /// fns alone (`summarize_within`'s out-of-reach declarations). A
    /// perspective added to the stdlib is added there too.
    #[test]
    fn the_copy_declares_no_perspective() {
        let program = crate::stdlib_bodies::program().expect("the stdlib parses");
        let perspectives: Vec<&str> = hale_syntax::ast::flat_decls(&program.items)
            .filter_map(|d| match d {
                TopDecl::Perspective(p) => Some(p.name.name.as_str()),
                _ => None,
            })
            .collect();
        assert!(perspectives.is_empty(), "the stdlib declares {perspectives:?}");
    }

    /// A program reaches what it names and no more: hello world reaches
    /// only what every program does, and naming a stdlib locus by its
    /// path reaches it and the declarations it names.
    #[test]
    fn a_program_reaches_what_it_names() {
        let hello = hale_syntax::parse_source("fn main() { println(\"hi\"); }").expect("parses");
        let base = StdlibReach::of(&[&hello]);
        assert!(!base.is_whole() && base.count() < StdlibReach::whole().count() / 4, "{} items", base.count());
        let client = hale_syntax::parse_source(
            "fn main() { let c = std::http::Client { }; let r = c.get(\"http://x\"); println(r.body); }",
        )
        .expect("parses");
        let reach = StdlibReach::of(&[&client]);
        let program = crate::stdlib_bodies::program().expect("the stdlib parses");
        let client_item = program
            .items
            .iter()
            .position(|d| matches!(d, TopDecl::Locus(l) if l.name.name == "__StdHttpClient"))
            .expect("the stdlib declares the http client");
        assert!(reach.contains(client_item) && !base.contains(client_item));
        assert!(base.items().all(|i| reach.contains(i)), "a reach grows with what the program names");
    }
}
