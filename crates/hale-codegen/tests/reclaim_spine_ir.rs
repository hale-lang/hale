//! The reclaim spine's IR, pinned per shape (F.40 phase 3, L4).
//!
//! Logical reclaim checks the instance's retirement guard, initiates its
//! accepted children's reclaims, cancels queued runs, then hands storage
//! to its release callback. The callback may defer while an admitted run
//! holds the instance. On its releasing path it waits for runs and flushes
//! retained children before releasing the arena and struct. These checks
//! follow both functions for roots, nested fields, replicas, pinned and
//! pool-placed fields, accepted children and elided arenas. The companion
//! `reclaim_cancel_ir` tests verify that the wait dominates every physical
//! release and that cancellation dominates every callback handoff, and pin
//! the guard in front of each runtime call: a step with nothing to do (no
//! run linked, nothing retired) is skipped on the words the function's
//! own first test reads, so a step here is located by its call, which
//! stays on the guard's other edge. The sequence is emitted once per
//! type: a site outside the type's own spine function calls the type's
//! `__reclaim_site_<L>_<spine>` or `__reclaim_field_<L>_<spine>`, so a
//! reclaim found there is counted at each call, under the caller's name.
//!
//! The dissolve cascade over the instance tree is pinned beside it, in
//! the order the plan places it (`LifecyclePlan::cascade_order` and
//! `cascade_fields`, line 12): an owner's fields drained in declaration
//! order, each before the owner's drain, a contract-typed field's
//! through the drain half its instantiation records (C32); the owner's
//! dissolve; the fields' dissolves and reclaims; a pinned locus's fields
//! drained on its thread before its `drain()` (C9).
//!
//! The order is read from the control flow, not the text: a step comes
//! before another when the other's block is reachable from its block and
//! not the reverse (or, in one block, by line).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use hale_codegen::build_executable_with_options;

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

/// Build `src` with its IR dumped; the IR.
fn ir(name: &str, src: &str) -> String {
    let program = hale_syntax::parse_source(src).unwrap_or_else(|e| panic!("{name}: parse: {e:?}"));
    let bin = harness::unique_bin(&format!("hale_reclaim_spine_{name}"));
    let ll: PathBuf = bin.with_extension("ll");
    let opts = hale_codegen::BuildOptions { dump_ir: Some(ll.clone()), ..build_opts::options() };
    build_executable_with_options(&program, &bin, &[], &opts).unwrap_or_else(|e| panic!("{name}: build: {e:?}"));
    let text = std::fs::read_to_string(&ll).unwrap_or_else(|e| panic!("{name}: read the IR: {e}"));
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(&ll);
    text
}

/// One function's blocks: label, lines, successors.
struct Func {
    name: String,
    blocks: Vec<(String, Vec<String>, Vec<String>)>,
}

fn functions(ir: &str) -> Vec<Func> {
    let mut out = Vec::new();
    let mut lines = ir.lines();
    while let Some(l) = lines.next() {
        if !l.starts_with("define") {
            continue;
        }
        let name = l.split('@').nth(1).and_then(|s| s.split('(').next()).unwrap_or("?").trim_matches('"').to_string();
        let mut blocks: Vec<(String, Vec<String>, Vec<String>)> = vec![("<entry>".into(), Vec::new(), Vec::new())];
        for l in lines.by_ref() {
            if l == "}" {
                break;
            }
            let head = l.split(';').next().unwrap_or("").trim_end();
            if !l.starts_with(' ') && head.ends_with(':') {
                let label = head.trim_end_matches(':').trim_matches('"').to_string();
                if blocks.len() == 1 && blocks[0].1.is_empty() {
                    blocks[0].0 = label;
                } else {
                    blocks.push((label, Vec::new(), Vec::new()));
                }
                continue;
            }
            let b = blocks.last_mut().expect("a block");
            let t = l.trim_start();
            if t.starts_with("br ") || t.starts_with("switch ") || t.starts_with("[") || t.starts_with("i64 ") {
                for part in t.split("label %").skip(1) {
                    let target: String = part
                        .trim_start_matches('"')
                        .chars()
                        .take_while(|c| !matches!(c, ',' | ' ' | ']' | '"'))
                        .collect();
                    b.2.push(target);
                }
            }
            b.1.push(l.to_string());
        }
        out.push(Func { name, blocks });
    }
    out
}

impl Func {
    fn index(&self) -> BTreeMap<&str, usize> {
        self.blocks.iter().enumerate().map(|(i, b)| (b.0.as_str(), i)).collect()
    }

    /// The blocks reachable from `from` (itself included only through a
    /// cycle).
    fn reach(&self, from: usize) -> BTreeSet<usize> {
        let idx = self.index();
        let mut seen = BTreeSet::new();
        let mut stack: Vec<usize> = self.blocks[from].2.iter().filter_map(|t| idx.get(t.as_str()).copied()).collect();
        while let Some(b) = stack.pop() {
            if seen.insert(b) {
                stack.extend(self.blocks[b].2.iter().filter_map(|t| idx.get(t.as_str()).copied()));
            }
        }
        seen
    }

    /// Every (block, line) whose line `pick` accepts.
    fn find(&self, pick: impl Fn(&str) -> bool) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for (b, (_, lines, _)) in self.blocks.iter().enumerate() {
            for (i, l) in lines.iter().enumerate() {
                if pick(l) {
                    out.push((b, i));
                }
            }
        }
        out
    }

    /// `a` runs before `b` on every path that reaches both.
    fn before(&self, a: (usize, usize), b: (usize, usize)) -> bool {
        if a.0 == b.0 {
            return a.1 < b.1;
        }
        self.reach(a.0).contains(&b.0) && !self.reach(b.0).contains(&a.0)
    }

    /// The nearest (block, line) after `from` that `pick` accepts, by
    /// breadth-first distance.
    fn next(&self, from: (usize, usize), pick: impl Fn(&str) -> bool) -> Option<(usize, usize)> {
        if let Some(i) = self.blocks[from.0].1.iter().skip(from.1 + 1).position(|l| pick(l)) {
            return Some((from.0, from.1 + 1 + i));
        }
        let idx = self.index();
        let mut seen = BTreeSet::from([from.0]);
        let mut queue: std::collections::VecDeque<usize> =
            self.blocks[from.0].2.iter().filter_map(|t| idx.get(t.as_str()).copied()).collect();
        while let Some(b) = queue.pop_front() {
            if !seen.insert(b) {
                continue;
            }
            if let Some(i) = self.blocks[b].1.iter().position(|l| pick(l)) {
                return Some((b, i));
            }
            queue.extend(self.blocks[b].2.iter().filter_map(|t| idx.get(t.as_str()).copied()));
        }
        None
    }
}

/// The reclaim steps of `locus` in every function that reclaims it,
/// each as the steps it shows in the order the control flow runs them:
/// (function, steps).
fn reclaims(ir: &str, locus: &str) -> Vec<(String, Vec<&'static str>)> {
    let fs = functions(ir);
    let mut out = Vec::new();
    for f in &fs {
        let guard = format!("%{locus}.storage.release.done");
        for guard in f.find(|l| l.trim_start().starts_with(&guard) && l.contains(" = icmp eq ptr ")) {
            let cancel = f.next(guard, |l| l.contains("@lotus_run_cancel_only("))
                .unwrap_or_else(|| panic!("{}: {locus} has no logical cancellation", f.name));
            let callback_prefix = format!("@__release_storage_{locus}_");
            let handoff = f.next(cancel, |l| l.contains(&callback_prefix))
                .unwrap_or_else(|| panic!("{}: {locus} has no storage callback", f.name));
            let mut logical = vec![("guard", guard)];
            let child = format!("(ptr %{locus}.cascade.child");
            if let Some(c) = f.find(|l| l.contains("@__reclaim_") && l.contains(&child))
                .into_iter().find(|&c| f.before(guard, c) && f.before(c, cancel)) {
                logical.push(("children", c));
            }
            logical.extend([("cancel", cancel), ("retain", handoff)]);
            for w in logical.windows(2) {
                assert!(f.before(w[0].1, w[1].1), "{}: {locus}'s {} must precede {}", f.name, w[0].0, w[1].0);
            }
            let line = &f.blocks[handoff.0].1[handoff.1];
            let start = line.find(&callback_prefix).expect("callback") + 1;
            let name = line[start..].split(['(', ')', ',', ' ']).next().expect("name");
            let release = func(&fs, name);
            let wait = release.find(|l| l.contains("@lotus_run_cancel_queued("));
            let flush = release.find(|l| l.contains("@lotus_reclaim_flush_owned("));
            assert_eq!(wait.len(), 1, "{name}: run wait");
            assert_eq!(flush.len(), 1, "{name}: descendant release");
            assert!(release.before(wait[0], flush[0]), "{name}: wait before descendants");
            let arenas = release.find(|l| ["@lotus_arena_destroy(", "@lotus_recpool_fixed_release(",
                "@lotus_recpool_slab_release("].iter().any(|p| l.contains(p)));
            let strukt = release.find(|l| l.contains("@lotus_child_struct_release("));
            assert_eq!(strukt.len(), 1, "{name}: struct release");
            assert!(release.before(flush[0], strukt[0]), "{name}: descendants before struct");
            for arena in &arenas {
                assert!(release.before(flush[0], *arena), "{name}: descendants before arena");
                assert!(release.before(*arena, strukt[0]), "{name}: arena before struct");
            }
            let mut steps: Vec<_> = logical.into_iter().map(|(name, _)| name).collect();
            steps.extend(["wait", "flush"]);
            if !arenas.is_empty() { steps.push("arena"); }
            steps.push("struct");
            for at in reclaiming(&fs, &f.name) {
                out.push((at, steps.clone()));
            }
        }
    }
    out
}

/// The functions a reclaim found in `name` is a reclaim of: `name`
/// itself, or, for a type's site or field function (the sequence every
/// site outside the type's own spine function calls), one entry per
/// call of it, under the calling function's name.
fn reclaiming(fs: &[Func], name: &str) -> Vec<String> {
    if !name.starts_with("__reclaim_site_") && !name.starts_with("__reclaim_field_") {
        return vec![name.to_string()];
    }
    let call = format!("@{name}(");
    let at: Vec<String> = fs
        .iter()
        .flat_map(|g| g.find(|l| l.contains(" call ") && l.contains(&call)).into_iter().map(|_| g.name.clone()))
        .collect();
    assert!(!at.is_empty(), "{name}: no site calls it");
    at
}

const FULL: &[&str] = &["guard", "children", "cancel", "retain", "wait", "flush", "arena", "struct"];
const CHILDLESS: &[&str] = &["guard", "cancel", "retain", "wait", "flush", "arena", "struct"];


/// `Sub` accepts a `Kid` (so its reclaim has children to reclaim before
/// releasing storage), which accepts an elidable `Leaf`.
const DECLS: &str = "locus Leaf { params { n: Int = 0; } }
locus Kid {
    params { n: Int = 0; }
    accept(c: Leaf) { }
    run() { Leaf { n: 1 }; println(\"ev kid\"); }
}
locus Sub {
    params { n: Int = 0; }
    accept(c: Kid) { }
    run() { Kid { }; }
    dissolve() { println(\"ev sub-dissolve\"); }
}
";

/// A pinned locus accepts nothing (`pinned locus declares accept()` is
/// refused), so the pinned shapes' `Sub` has no children to reclaim.
const PINNED_DECLS: &str = "locus Sub {
    params { n: Int = 0; }
    run() { println(\"ev sub-run\"); }
    dissolve() { println(\"ev sub-dissolve\"); }
}
";

/// Every reclaim of `locus` in the module, held to `want`; the functions
/// they are in.
fn assert_every(ir: &str, locus: &str, want: &[&str]) -> BTreeSet<String> {
    let all = reclaims(ir, locus);
    assert!(!all.is_empty(), "no reclaim of {locus} in the IR");
    for (f, steps) in &all {
        assert_eq!(steps, want, "{f}: {locus}'s reclaim");
    }
    all.into_iter().map(|(f, _)| f).collect()
}

#[test]
fn a_root_childs_reclaim_reads_the_plans_order() {
    let src = format!("{DECLS}main locus App {{ params {{ s: Sub = Sub {{ }}; }} }}\nfn main() {{ App {{ }}; }}\n");
    let ir = ir("root_child", &src);
    let fns = assert_every(&ir, "Sub", FULL);
    assert!(fns.contains("main"), "fn main's teardown reclaims the root's field: {fns:?}");
}

#[test]
fn a_nested_childs_reclaim_reads_the_plans_order() {
    let src = format!(
        "{DECLS}locus Mid {{ params {{ s: Sub = Sub {{ }}; }} }}\nmain locus App {{ params {{ m: Mid = Mid {{ }}; }} }}\nfn main() {{ App {{ }}; }}\n"
    );
    let ir = ir("nested_child", &src);
    let fns = assert_every(&ir, "Sub", FULL);
    assert!(fns.contains("main") && fns.contains("__reclaim_Mid"), "{fns:?}");
}

/// Each replica's per-entry teardown, after its join.
#[test]
fn each_replicas_reclaim_reads_the_plans_order() {
    let src = format!(
        "{PINNED_DECLS}main locus App {{\n    params {{ s: Sub = Sub {{ }}; }}\n    placement {{ s: pinned(replicas = 2); }}\n}}\nfn main() {{ App {{ }}; }}\n"
    );
    let ir = ir("replica", &src);
    let all = reclaims(&ir, "Sub");
    assert!(all.iter().filter(|(f, _)| f == "main").count() >= 2, "a reclaim per replica in fn main: {all:?}");
    for (f, steps) in &all {
        assert_eq!(steps, CHILDLESS, "{f}");
    }
}

#[test]
fn a_pinned_childs_reclaim_reads_the_plans_order() {
    let src = format!(
        "{PINNED_DECLS}main locus App {{\n    params {{ s: Sub = Sub {{ }}; }}\n    placement {{ s: pinned; }}\n}}\nfn main() {{ App {{ }}; }}\n"
    );
    let ir = ir("pinned_child", &src);
    let fns = assert_every(&ir, "Sub", CHILDLESS);
    assert!(fns.contains("main"), "{fns:?}");
}

/// The case the cancellation's position is for: a run still queued on a
/// pool is canceled before anything of the child is released.
#[test]
fn a_pool_placed_childs_reclaim_reads_the_plans_order() {
    let src = format!(
        "{DECLS}main locus App {{\n    params {{ s: Sub = Sub {{ }}; }}\n    placement {{ s: cooperative(pool = side); }}\n}}\nfn main() {{ App {{ }}; }}\n"
    );
    let ir = ir("pool_child", &src);
    let fns = assert_every(&ir, "Sub", FULL);
    assert!(fns.contains("main"), "{fns:?}");
}

/// An accepted child's reclaim spine (`__reclaim_<L>`, from its run's end
/// and its owner's cascade), and an elided-arena child's: no arena of its
/// own to release, the struct's release after the cancellation.
#[test]
fn an_accepted_and_an_elided_childs_reclaim_read_the_plans_order() {
    let src = format!("{DECLS}main locus App {{ params {{ s: Sub = Sub {{ }}; }} }}\nfn main() {{ App {{ }}; }}\n");
    let ir = ir("accepted", &src);
    let fns = assert_every(&ir, "Kid", FULL);
    assert!(fns.contains("__reclaim_Kid"), "{fns:?}");
    let fns = assert_every(&ir, "Leaf", &["guard", "cancel", "retain", "wait", "flush", "struct"]);
    assert!(fns.contains("__reclaim_Leaf"), "{fns:?}");
}

/// The steps `marks` names, in `f`, ordered by the control flow: each
/// mark is (name, the substrings, `&&`-joined, of the one line that is
/// its step).
fn ordered(f: &Func, marks: &[(&'static str, String)]) -> Vec<&'static str> {
    let mut steps: Vec<(&'static str, (usize, usize))> = Vec::new();
    for (name, pat) in marks {
        let found = f.find(|l| pat.split(" && ").all(|p| l.contains(p)));
        assert_eq!(found.len(), 1, "{}: `{pat}` ({name}) is not one line: {found:?}", f.name);
        steps.push((name, found[0]));
    }
    steps.sort_by(|a, b| if f.before(a.1, b.1) { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater });
    for w in steps.windows(2) {
        assert!(f.before(w[0].1, w[1].1), "{}: {} and {} are not ordered by the control flow", f.name, w[0].0, w[1].0);
    }
    steps.into_iter().map(|(n, _)| n).collect()
}

fn func<'a>(fs: &'a [Func], name: &str) -> &'a Func {
    fs.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("no function {name}"))
}

/// C32: a contract-typed field drains before its owner's drain, through
/// the drain half of its recorded pair; the rest of its spine runs after
/// the owner's dissolve.
#[test]
fn a_contract_fields_drain_runs_before_its_owners() {
    let src = "interface Probe { fn v() -> Int; }
locus Kid {
    fn v() -> Int { return 1; }
    drain() { println(\"ev kid-drain\"); }
    dissolve() { println(\"ev kid-dissolve\"); }
}
main locus App {
    params { k: Probe = Kid { }; }
    drain() { println(\"ev app-drain\"); }
    dissolve() { println(\"ev app-dissolve\"); }
}
fn main() { App { }; }
";
    let ir = ir("contract_field", src);
    let fs = functions(&ir);
    let half = |n: u32| format!("%App.k.contract.half.ptr && getelementptr inbounds [2 x ptr] && i32 0, i32 {n}");
    // fn main reclaims App through App's site function; __reclaim_App
    // holds its own.
    for (name, reclaim) in [("main", "call void @__reclaim_site_App_"), ("__reclaim_App", "%App.storage.release.done = ")] {
        let f = func(&fs, name);
        let marks = [
            ("field drain", half(0)),
            ("drain", "call void @App.drain(".to_string()),
            ("dissolve", "call void @App.dissolve(".to_string()),
            ("field rest", half(1)),
            ("reclaim", reclaim.to_string()),
        ];
        assert_eq!(ordered(f, &marks), ["field drain", "drain", "dissolve", "field rest", "reclaim"], "{name}");
    }
    // The pair: Kid's drain half drains it once (latched), the rest is
    // its spine without the drain.
    assert!(ir.contains("@__contract_teardown_Kid = internal constant [2 x ptr] [ptr @__drain_Kid, ptr @__reclaim_drained_Kid]"));
    let drain = func(&fs, "__drain_Kid");
    assert_eq!(drain.find(|l| l.contains("call void @Kid.drain(")).len(), 1);
    let rest = func(&fs, "__reclaim_drained_Kid");
    assert!(rest.find(|l| l.contains("call void @Kid.drain(")).is_empty(), "the rest does not drain again");
    assert_eq!(rest.find(|l| l.contains("call void @Kid.dissolve(")).len(), 1);
}

/// C9: a pinned locus's thread drains its owned fields before its own
/// drain().
#[test]
fn a_pinned_locus_drains_its_fields_on_its_thread_first() {
    let src = "locus Inner { drain() { println(\"ev inner-drain\"); } }
locus Outer {
    params { i: Inner = Inner { }; }
    run() { println(\"ev outer-run\"); }
    drain() { println(\"ev outer-drain\"); }
}
main locus App {
    params { o: Outer = Outer { }; }
    placement { o: pinned; }
}
fn main() { App { }; }
";
    let ir = ir("pinned_fields", src);
    let fs = functions(&ir);
    let f = func(&fs, "__pinned_main_Outer");
    let marks = [("field drain", "call void @Inner.drain(".to_string()), ("drain", "call void @Outer.drain(".to_string())];
    assert_eq!(ordered(f, &marks), ["field drain", "drain"]);
}

/// Line 12 over the tree: fields in declaration order (not by name),
/// each drained before the owner's drain and dissolved after its
/// dissolve, each logical teardown started before the next; storage may be retained.
#[test]
fn fields_are_torn_down_in_declaration_order() {
    let src = "locus Kid {
    params { n: Int = 0; }
    drain() { println(\"ev kid-drain \", self.n); }
    dissolve() { println(\"ev kid-dissolve \", self.n); }
}
main locus App {
    params { z: Kid = Kid { n: 1 }; a: Kid = Kid { n: 2 }; m: Kid = Kid { n: 3 }; }
    drain() { println(\"ev app-drain\"); }
    dissolve() { println(\"ev app-dissolve\"); }
}
fn main() { App { }; }
";
    let ir = ir("declaration_order", src);
    let fs = functions(&ir);
    let f = func(&fs, "main");
    let mut marks: Vec<(&'static str, String)> = Vec::new();
    for (step, field) in [("z drain", "z"), ("a drain", "a"), ("m drain", "m")] {
        marks.push((step, ["call void @Kid.drain(ptr %App.", field, ".drain.load"].concat()));
    }
    marks.push(("drain", "call void @App.drain(".to_string()));
    marks.push(("dissolve", "call void @App.dissolve(".to_string()));
    for (step, field) in [("z dissolve", "z"), ("a dissolve", "a"), ("m dissolve", "m")] {
        marks.push((step, ["call void @Kid.dissolve(ptr %App.", field, ".cascade.load"].concat()));
    }
    assert_eq!(
        ordered(f, &marks),
        ["z drain", "a drain", "m drain", "drain", "dissolve", "z dissolve", "a dissolve", "m dissolve"]
    );
}
