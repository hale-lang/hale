//! Decision line 19 at each emitted teardown spine: logical reclaim
//! cancels queued runs before handing storage to its release callback.
//! The callback waits for admitted runs before releasing any storage,
//! including forms, recognition pools, descendants, arenas and structs.
//! Its deferred return touches none of those resources.
//!
//! These checks follow the pre-optimization CFG in both functions. They
//! cover eager, frame, explicit return, reclaim and field-cascade paths;
//! executable fixtures separately exercise the deferred callback.

#[path = "support/harness.rs"]
mod harness;
#[path = "support/build.rs"]
mod build_opts;

const KID: &str = "locus Kid {
    params { tag: Int = 0; }
    run() { println(\"ev kid-run \" + to_string(self.tag)); }
    dissolve() { println(\"ev kid-dissolve \" + to_string(self.tag)); }
    fn v() -> Int { return self.tag; }
}
";

const CANCEL: &str = "call void @lotus_run_cancel_queued(ptr ";

fn ir_of(tag: &str, body: &str) -> String {
    let src = [KID, body].concat();
    let program = hale_syntax::parse_source(&src).unwrap_or_else(|e| panic!("{tag}: parse: {e:?}\n{src}"));
    let bin = harness::unique_bin(&format!("reclaim_cancel_ir_{tag}"));
    let ir = harness::build_ir_text(&program, &bin).unwrap_or_else(|e| panic!("{tag}: build: {e:?}\n{src}"));
    let _ = std::fs::remove_file(&bin);
    ir
}

/// The body of `define ... @<name>(`, through its closing brace.
fn function<'a>(ir: &'a str, name: &str) -> &'a str {
    let needle = format!(" @{name}(");
    let start = ir
        .match_indices("define ")
        .map(|(i, _)| i)
        .find(|&i| ir[i..].lines().next().is_some_and(|l| l.contains(&needle)))
        .unwrap_or_else(|| panic!("`{name}` is not defined in the IR"));
    let end = ir[start..].find("\n}\n").map(|i| start + i + 2).unwrap_or(ir.len());
    &ir[start..end]
}

/// A function's basic blocks: label, body, and the labels it branches
/// to. The entry block, unlabeled, is `""`.
struct Block<'a> {
    label: &'a str,
    body: String,
    succs: Vec<&'a str>,
}

fn blocks(f: &str) -> Vec<Block<'_>> {
    let mut out: Vec<Block<'_>> = vec![Block { label: "", body: String::new(), succs: Vec::new() }];
    for line in f.lines().skip(1) {
        if line == "}" {
            break;
        }
        if !line.starts_with(' ') && !line.is_empty() {
            if let Some((label, _)) = line.split_once(':') {
                out.push(Block { label: label.trim_matches('"'), body: String::new(), succs: Vec::new() });
                continue;
            }
        }
        let b = out.last_mut().expect("the entry block");
        b.body.push_str(line);
        b.body.push('\n');
        for (i, _) in line.match_indices("label %") {
            let name = &line[i + "label %".len()..];
            let end = name.find([',', ' ', ']']).unwrap_or(name.len());
            b.succs.push(name[..end].trim_matches('"'));
        }
    }
    out
}

const RELEASES: &[&str] = &[
    "@lotus_arena_destroy(", "@lotus_recpool_fixed_release(",
    "@lotus_recpool_slab_release(", "@lotus_child_struct_release(",
    "@lotus_children_free(", "@lotus_vec_destroy(", "@lotus_hashmap_destroy(",
    "@lotus_ring_buffer_destroy(", "@lotus_lru_free(",
    "@lotus_recpool_fixed_destroy(", "@lotus_recpool_slab_destroy(",
    "@lotus_reclaim_flush_owned(",
];

/// Every path that reaches a physical release must first wait. The
/// alternative path may return with retirement pending, without freeing.
fn assert_wait_dominates_release(tag: &str, f: &str) {
    let bs = blocks(f);
    let first = bs.iter().position(|b| !b.body.is_empty()).expect("entry block");
    let mut work = vec![(first, false)];
    let mut seen = Vec::new();
    let mut waits = 0;
    let mut releases = 0;
    while let Some((index, mut waited)) = work.pop() {
        if seen.contains(&(index, waited)) { continue; }
        seen.push((index, waited));
        let b = &bs[index];
        for line in b.body.lines() {
            if line.contains(CANCEL) { waited = true; waits += 1; }
            if RELEASES.iter().any(|r| line.contains(r)) {
                releases += 1;
                assert!(waited, "{tag}: release without run-hold wait in {}: {line}", b.label);
            }
        }
        for succ in &b.succs {
            let next = bs.iter().position(|b| b.label == *succ).expect("successor");
            work.push((next, waited));
        }
    }
    assert!(waits > 0 && releases > 0, "{tag}: no wait/release path: {f}");
}

fn assert_spine(tag: &str, ir: &str, func: &str, l: &str) -> usize {
    let f = function(ir, func);
    let bs = blocks(f);
    let prefix = format!("{l}.storage.release.live");
    let starts: Vec<_> = bs.iter().enumerate().filter(|(_, b)| b.label.strip_prefix(&prefix)
        .is_some_and(|r| r.chars().all(|c| c.is_ascii_digit()))).map(|(i, _)| i).collect();
    assert!(!starts.is_empty(), "{tag}: `{func}` holds no reclaim of {l}:\n{f}");
    let helper_prefix = format!("@__release_storage_{l}_");
    let mut helpers = Vec::new();
    for start in &starts {
        let mut work = vec![(*start, false)];
        let mut seen = Vec::new();
        let mut reached = false;
        while let Some((index, mut canceled)) = work.pop() {
            if seen.contains(&(index, canceled)) { continue; }
            seen.push((index, canceled));
            let b = &bs[index];
            let mut handed_off = false;
            for line in b.body.lines() {
                if line.contains("call void @lotus_run_cancel_only(") { canceled = true; }
                if let Some(at) = line.find(&helper_prefix) {
                    assert!(canceled, "{tag}: release callback before cancellation: {line}");
                    let name = &line[at + 1..];
                    let end = name.find(['(', ')', ',', ' ']).expect("callee boundary");
                    let name = &name[..end];
                    if !helpers.contains(&name) { helpers.push(name); }
                    handed_off = true;
                    reached = true;
                }
            }
            if !handed_off {
                for succ in &b.succs {
                    let next = bs.iter().position(|b| b.label == *succ).expect("successor");
                    work.push((next, canceled));
                }
            }
        }
        assert!(reached, "{tag}: logical reclaim never reaches its storage callback");
    }
    for helper in helpers { assert_wait_dominates_release(tag, function(ir, helper)); }
    starts.len()
}

/// The eager spine: a statement literal is torn down where it stands.
#[test]
fn eager_spine_cancels_before_the_arena_goes() {
    let ir = ir_of("eager", "fn main() { Kid { tag: 1 }; }\n");
    assert_spine("eager", &ir, "main", "Kid");
}

/// The deferred spine: a `let`-bound literal at its frame's flush.
#[test]
fn deferred_spine_cancels_before_the_arena_goes() {
    let ir = ir_of("deferred", "fn work() {\n    let k = Kid { tag: 1 };\n    println(\"ev v \" + to_string(k.v()));\n}\n\nfn main() { work(); }\n");
    assert_spine("deferred", &ir, "work", "Kid");
}

/// The return spine: `return` from `fn main` tears down what main
/// owns on that path, and the fall-through keeps its own copy.
#[test]
fn return_spine_cancels_before_the_arena_goes() {
    let ir = ir_of(
        "return",
        "fn main() {\n    let k = Kid { tag: 1 };\n    if k.v() == 1 {\n        return;\n    }\n    println(\"ev after\");\n}\n",
    );
    let exits = assert_spine("return", &ir, "main", "Kid");
    assert!(exits >= 2, "return: main reclaims Kid on {exits} exit(s), not on the return and the fall-through");
}

/// The reclaim spine: a flow child's run end reclaims it through
/// `__reclaim_<L>`.
#[test]
fn reclaim_spine_cancels_before_the_arena_goes() {
    let ir = ir_of(
        "reclaim",
        "locus Host {\n    accept(c: Kid) { }\n    release (c: Kid) { }\n    run() { Kid { tag: 1 }; }\n}\n\nfn main() { Host { }; }\n",
    );
    assert_spine("reclaim", &ir, "__reclaim_Kid", "Kid");
}

/// The dissolve cascade: an owner's field is reclaimed inside the
/// owner's teardown, before the owner's own arena.
#[test]
fn dissolve_cascade_cancels_before_the_arena_goes() {
    let ir = ir_of("cascade", "locus Box {\n    params { k: Kid = Kid { tag: 1 }; }\n}\n\nfn main() { Box { }; }\n");
    assert_spine("cascade", &ir, "main", "Kid");
    assert_spine("cascade", &ir, "main", "Box");
}
