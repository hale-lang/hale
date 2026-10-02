//! Decision line 19 in the emitted IR (F.40 phase 3, L5): every reclaim
//! path begins its Reclaim bracket with `lotus_run_cancel_queued(self)`,
//! past the `__arena` latch and before the arena (or, for an elided
//! arena, the struct) is released, so a run queued on any pool for the
//! instance finds it whole or finds its ticket canceled.
//!
//! The teardown spines all funnel into one chokepoint
//! (`emit_locus_arena_destroy`), so the call is one line in the
//! compiler; what this file pins is that each spine reaches it: the
//! eager spine (a statement literal), the deferred spine (a `let` at the
//! frame flush), the return spine (`return` from `fn main`), the reclaim
//! spine (`__reclaim_<L>`, a flow child's run end) and the dissolve
//! cascade (an owner's field). For each, the pre-optimization IR of the
//! spine's function holds the latch-passed block of the instance's
//! reclaim, and that block calls the cancel before it branches to the
//! release; across the module there is exactly one cancel per reclaim
//! block, so the IR changes by that call and nothing else (the release
//! IR shadow over the corpus says the same of every program).
//!
//! Programs are assembled from ordinary string constants, never a raw
//! string literal (the corpus harvest's note, `lifecycle_matrix.rs`).

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

/// The latch-passed blocks of locus `l`'s reclaims in `f`: the arena's
/// release path and the elided arena's struct release.
fn latch_passed<'a>(bs: &'a [Block<'a>], l: &str) -> Vec<&'a Block<'a>> {
    let numbered = |label: &str, prefix: &str| {
        label.strip_prefix(prefix).is_some_and(|r| r.chars().all(|c| c.is_ascii_digit()))
    };
    let (arena, elide) = (format!("{l}.arena.destroy.do"), format!("{l}.elide.release_struct"));
    bs.iter().filter(|b| numbered(b.label, &arena) || numbered(b.label, &elide)).collect()
}

const RELEASES: &[&str] =
    &["@lotus_arena_destroy(", "@lotus_recpool_fixed_release(", "@lotus_recpool_slab_release(", "@lotus_child_struct_release("];

/// The latch's bookkeeping, the only calls allowed before the cancel:
/// the drain-observer count (GH #1077), and in a trace build the
/// Reclaim's entry.
fn bookkeeping(line: &str) -> bool {
    line.contains("@lotus_drain_observer_add(") || line.contains("@lotus_lc_")
}

/// From the latch-passed block, every path to a release passes through
/// the cancel, and the cancel is reached: the walk that stops at the
/// cancel's block finds no release, and finds that block. Before the
/// cancel only the latch's [`bookkeeping`] calls anything.
fn assert_cancels_first(tag: &str, bs: &[Block<'_>], start: &Block<'_>) {
    let by_label = |l: &str| bs.iter().find(|b| b.label == l).unwrap_or_else(|| panic!("{tag}: no block {l}"));
    let mut seen: Vec<&str> = vec![start.label];
    let mut work = vec![start];
    let mut cancels = 0;
    while let Some(b) = work.pop() {
        if let Some(at) = b.body.find(CANCEL) {
            cancels += 1;
            let before = &b.body[..at];
            assert!(!RELEASES.iter().any(|r| before.contains(r)), "{tag}: {} releases before it cancels:\n{}", b.label, b.body);
            let calls: Vec<&str> = before.lines().filter(|l| l.contains("call ") && !bookkeeping(l)).collect();
            assert!(calls.is_empty(), "{tag}: {} calls {calls:#?} before the cancel", b.label);
            continue;
        }
        assert!(!RELEASES.iter().any(|r| b.body.contains(r)), "{tag}: {} reaches a release with no cancel:\n{}", b.label, b.body);
        let calls: Vec<&str> = b.body.lines().filter(|l| l.contains("call ") && !bookkeeping(l)).collect();
        assert!(calls.is_empty(), "{tag}: {} calls {calls:#?} before the cancel", b.label);
        for s in &b.succs {
            if !seen.contains(s) {
                seen.push(s);
                work.push(by_label(s));
            }
        }
    }
    assert_eq!(cancels, 1, "{tag}: from {} the cancel is reached {cancels} times, not once", start.label);
}

/// The spine's function holds `l`'s reclaim, each one cancels first,
/// and the module carries exactly one cancel per reclaim.
fn assert_spine(tag: &str, ir: &str, func: &str, l: &str) -> usize {
    let bs = blocks(function(ir, func));
    let mine = latch_passed(&bs, l);
    assert!(!mine.is_empty(), "{tag}: `{func}` holds no reclaim of {l}:\n{}", function(ir, func));
    for b in &mine {
        assert_cancels_first(tag, &bs, b);
    }
    // Every locus's reclaim, the stdlib's included.
    let any_reclaim = |label: &str| {
        [".arena.destroy.do", ".elide.release_struct"].iter().any(|k| {
            label.rfind(k).is_some_and(|i| label[i + k.len()..].chars().all(|c| c.is_ascii_digit()))
        })
    };
    let reclaims: usize =
        ir.split("\ndefine ").skip(1).map(|f| blocks(f).iter().filter(|b| any_reclaim(b.label)).count()).sum();
    assert_eq!(ir.matches(CANCEL).count(), reclaims, "{tag}: the module's cancels and its reclaims differ");
    mine.len()
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
