//! Decision line 19 at each emitted teardown spine: logical reclaim
//! cancels queued runs before handing storage to its release callback.
//! The callback waits for admitted runs before releasing any storage,
//! including forms, recognition pools, descendants, arenas and structs.
//! Its deferred return touches none of those resources.
//!
//! These checks follow the pre-optimization CFG in both functions. They
//! cover eager, frame, explicit return, reclaim and field-cascade paths;
//! executable fixtures separately exercise the deferred callback.
//!
//! Each runtime call of the reclaim sits behind a guard that skips it
//! when it has nothing to do: `br i1 %<tag>.idle, label %<tag>.after,
//! label %<tag>.call`, where `<tag>.idle` is the conjunction of
//! `load atomic ... acquire` of the runtime words the function's own
//! first test reads, each equal to zero. The idle edge is that step
//! with nothing to do, so a path through it has canceled (or waited)
//! as surely as one through the call; the guard's words are pinned
//! here, so a guard that reads less than its function cannot pass.
//!
//! The guarded sequence is emitted once per locus type, not at each
//! site: a site outside the type's own spine function calls the type's
//! `__reclaim_site_<L>_<spine>` (or, for a field under its owner,
//! `__reclaim_field_<L>_<spine>`), a cascade's skip calls
//! `__reclaim_live_<L>`, and a cascade's reclaim scope is entered and
//! left through the shared `__reclaim_scope_enter` / `_leave`. A site
//! holds the call and no protocol call of its own; the checks follow
//! the call into the function that holds the guards.

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

/// The words `lotus_run_cancel_queued`'s first test reads.
const WAIT_WORDS: &[&str] = &["lotus_run_tickets_live"];
/// `lotus_run_cancel_only`'s: its cancel's, then its forget's.
const CANCEL_ONLY_WORDS: &[&str] = &["lotus_run_tickets_live", "lotus_owner_domain_count"];
/// `lotus_reclaim_flush_owned`'s.
const FLUSH_WORDS: &[&str] = &["lotus_reclaim_records_live"];
/// `lotus_reclaim_defer`'s and `lotus_reclaim_release_enter`'s.
const QUIET_WORDS: &[&str] = &["lotus_run_tickets_live", "lotus_reclaim_records_live"];

/// The value a line defines, `%<name> = ...`: (name, right-hand side).
fn definition<'a>(f: &'a str, name: &str) -> &'a str {
    let head = format!("%{name} = ");
    f.lines()
        .find_map(|l| l.trim_start().strip_prefix(&head))
        .unwrap_or_else(|| panic!("`%{name}` is not defined:\n{f}"))
}

/// The runtime words the guard test `%cond` reads, in order: `cond` is
/// an `and i1` of `icmp eq i64 %w, 0`, each `%w` an acquire load of a
/// runtime word.
fn guard_words(f: &str, cond: &str) -> Vec<String> {
    let rhs = definition(f, cond);
    if let Some(ops) = rhs.strip_prefix("and i1 ") {
        return ops
            .split(", ")
            .flat_map(|op| guard_words(f, op.trim().trim_start_matches('%')))
            .collect();
    }
    let ops = rhs
        .strip_prefix("icmp eq i64 ")
        .unwrap_or_else(|| panic!("guard `%{cond}` is not a zero test: {rhs}"));
    let (word, zero) = ops.split_once(", ").expect("two operands");
    assert_eq!(zero.trim(), "0", "guard `%{cond}` compares with zero: {rhs}");
    let load = definition(f, word.trim_start_matches('%'));
    let read = load
        .strip_prefix("load atomic i64, ptr @")
        .and_then(|r| r.strip_suffix(" acquire, align 8"))
        .unwrap_or_else(|| panic!("guard `%{cond}` reads a word other than by an acquire load: {load}"));
    vec![read.to_string()]
}

/// The conditional branch whose second edge is `label`, without its
/// metadata (a guard's branch weighs its idle edge: `, !prof !<n>`).
fn branch_to<'a>(f: &'a str, label: &str) -> Option<&'a str> {
    f.lines()
        .map(|l| l.trim().split(", !").next().unwrap_or(""))
        .find(|l| l.starts_with("br i1 %") && l.ends_with(&format!("label %{label}")))
}

/// The guard in front of the block `call_label` (`<tag>.call`, LLVM
/// numbering repeats): its idle test, and the words that reads. Its
/// idle edge goes to `<tag>.after`.
fn guard_of(f: &str, call_label: &str) -> (String, Vec<String>) {
    let tag = call_label
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .strip_suffix(".call")
        .unwrap_or_else(|| panic!("`{call_label}` is not a guarded call block"));
    let br = branch_to(f, call_label).unwrap_or_else(|| panic!("nothing branches to `{call_label}`"));
    let cond = br["br i1 %".len()..].split(',').next().expect("condition");
    assert!(cond.starts_with(&format!("{tag}.idle")), "`{call_label}` is entered on `{cond}`, not its idle test: {br}");
    let after = br.split("label %").nth(1).expect("the idle edge").trim_end_matches(", ");
    assert!(after.starts_with(&format!("{tag}.after")), "the idle edge of `{call_label}` goes to `{after}`: {br}");
    (cond.to_string(), guard_words(f, cond))
}

/// Every call of `callee` in `f` sits alone behind a guard reading
/// exactly `words`; the guards' idle tests, by name.
fn assert_guarded(tag: &str, f: &str, callee: &str, words: &[&str]) -> Vec<String> {
    let bs = blocks(f);
    let mut conds = Vec::new();
    for b in &bs {
        let calls = b.body.lines().filter(|l| l.contains(&format!("@{callee}("))).count();
        if calls == 0 {
            continue;
        }
        assert_eq!(calls, 1, "{tag}: `{}` holds {calls} calls of {callee}", b.label);
        let (cond, read) = guard_of(f, b.label);
        assert_eq!(read, words, "{tag}: the guard of {callee} in `{}`", b.label);
        conds.push(cond);
    }
    assert!(!conds.is_empty(), "{tag}: no call of {callee}");
    conds
}

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

/// The guard's branch on one of `conds`, its idle tests.
fn branches_on(line: &str, conds: &[String]) -> bool {
    line.trim_start()
        .strip_prefix("br i1 %")
        .and_then(|r| r.split(',').next())
        .is_some_and(|c| conds.iter().any(|g| c == g))
}

/// Every path that reaches a physical release must first wait (call
/// the wait, or pass its guard, which found no run to wait for). The
/// alternative path may return with retirement pending, without freeing.
fn assert_wait_dominates_release(tag: &str, f: &str) {
    let waits_idle = assert_guarded(tag, f, "lotus_run_cancel_queued", WAIT_WORDS);
    assert_guarded(tag, f, "lotus_reclaim_flush_owned", FLUSH_WORDS);
    assert_guarded(tag, f, "lotus_reclaim_defer", QUIET_WORDS);
    assert_guarded(tag, f, "lotus_reclaim_release_enter", QUIET_WORDS);
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
            if line.contains(CANCEL) || branches_on(line, &waits_idle) { waited = true; waits += 1; }
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

/// The functions `f` calls whose names begin with `prefix`, one entry
/// per call.
fn calls_of<'a>(f: &'a str, prefix: &str) -> Vec<&'a str> {
    let needle = format!("@{prefix}");
    f.lines()
        .filter(|l| l.contains(" call "))
        .filter_map(|l| {
            let at = l.find(&needle)?;
            let name = &l[at + 1..];
            Some(&name[..name.find('(').expect("an argument list")])
        })
        .collect()
}

/// The protocol's runtime calls, each of which a site leaves to its
/// type's functions.
const PROTOCOL: &[&str] = &[
    "@lotus_reclaim_pending(", "@lotus_run_cancel_only(", "@lotus_run_cancel_queued(",
    "@lotus_reclaim_flush_owned(", "@lotus_reclaim_defer(", "@lotus_reclaim_release_enter(",
    "@lotus_reclaim_release_leave(", "@lotus_reclaim_scope_enter(", "@lotus_reclaim_scope_leave(",
];

/// `func`'s reclaims of `l`, each held to the cancellation before its
/// storage callback and the wait before every release: in place in the
/// type's own spine function `__reclaim_<l>`; elsewhere, `func` calls
/// the type's site (or field) function, which holds the sequence, and
/// makes no protocol call itself. The number of reclaims.
fn assert_spine(tag: &str, ir: &str, func: &str, l: &str) -> usize {
    if func == format!("__reclaim_{l}") {
        return assert_spine_in(tag, ir, func, l);
    }
    let f = function(ir, func);
    for call in PROTOCOL {
        assert!(!f.contains(call), "{tag}: `{func}` makes a protocol call its type's function holds: {call}");
    }
    let mut sites = calls_of(f, &format!("__reclaim_site_{l}_"));
    sites.extend(calls_of(f, &format!("__reclaim_field_{l}_")));
    assert!(!sites.is_empty(), "{tag}: `{func}` calls no site function of {l}:\n{f}");
    let mut seen = Vec::new();
    for site in &sites {
        if !seen.contains(site) {
            seen.push(*site);
            assert_eq!(assert_spine_in(tag, ir, site, l), 1, "{tag}: `{site}` is one reclaim of {l}");
        }
    }
    sites.len()
}

fn assert_spine_in(tag: &str, ir: &str, func: &str, l: &str) -> usize {
    let f = function(ir, func);
    let cancels_idle = assert_guarded(tag, f, "lotus_run_cancel_only", CANCEL_ONLY_WORDS);
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
                if line.contains("call void @lotus_run_cancel_only(") || branches_on(line, &cancels_idle) {
                    canceled = true;
                }
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

/// Every `lotus_reclaim_pending` call in `f` sits behind its guard: with
/// no record alive (`lotus_reclaim_records_live`, acquire) the guard
/// answers from the instance's claim word itself, an acquire load equal
/// to `LOTUS_RECLAIM_CLAIMED` (1), to the same two places the call's
/// answer goes.
fn assert_pending_guarded(tag: &str, f: &str) -> usize {
    let bs = blocks(f);
    let mut guarded = 0;
    for b in bs.iter().filter(|b| b.body.contains("@lotus_reclaim_pending(")) {
        let call_tag = b.label.trim_end_matches(|c: char| c.is_ascii_digit()).strip_suffix(".call")
            .unwrap_or_else(|| panic!("{tag}: `{}` calls lotus_reclaim_pending unguarded", b.label));
        let br = branch_to(f, b.label).unwrap_or_else(|| panic!("{tag}: nothing branches to `{}`", b.label));
        let cond = br["br i1 %".len()..].split(',').next().expect("condition");
        assert!(cond.starts_with(&format!("{call_tag}.idle")), "{tag}: `{}` entered on `{cond}`", b.label);
        assert_eq!(guard_words(f, cond), ["lotus_reclaim_records_live"], "{tag}: the pending guard's words");
        let claim_label = br.split("label %").nth(1).expect("the idle edge").trim_end_matches(", ");
        if b.body.contains("@lotus_reclaim_pending(ptr ") && b.body.contains(", ptr null)") {
            // The winning spine's own storage step passes no claim: the
            // function answers "not retired" at once.
            assert_eq!(Some(&claim_label), b.succs.get(1), "{tag}: a null claim's idle edge is the call's no");
            guarded += 1;
            continue;
        }
        let claim = bs.iter().find(|c| c.label == claim_label).expect("the claim block");
        assert!(claim.label.starts_with(&format!("{call_tag}.claim")), "{tag}: the idle edge goes to `{}`", claim.label);
        let word = claim.body.lines().find_map(|l| l.trim_start().split_once(" = load atomic i64, ptr %"))
            .unwrap_or_else(|| panic!("{tag}: `{}` loads no claim word:\n{}", claim.label, claim.body));
        assert!(word.1.ends_with(" acquire, align 8"), "{tag}: the claim word is read acquire: {}", word.1);
        assert!(claim.body.contains(&format!("= icmp eq i64 {}, 1", word.0)), "{tag}: the guard tests CLAIMED:\n{}", claim.body);
        assert_eq!(claim.succs, b.succs, "{tag}: the guard and the call answer to the same places");
        guarded += 1;
    }
    guarded
}

/// The retirement check every reclaim begins with reads the claim
/// inline when nothing is retired, on each spine: in the type's spine
/// function, in its site function (the let-bound reclaim's), and in its
/// live test (an eager literal's skip and an owner's cascade over its
/// field), which the sites call instead of checking for themselves.
#[test]
fn the_pending_check_reads_the_claim_when_nothing_is_retired() {
    let ir = ir_of("pending", "fn work() {\n    let k = Kid { tag: 1 };\n    println(\"ev v \" + to_string(k.v()));\n}\n\nfn main() { work(); }\n");
    let work = function(&ir, "work");
    assert!(!work.contains("@lotus_reclaim_pending("), "pending: work checks Kid's claim in Kid's site function:\n{work}");
    let sites = calls_of(work, "__reclaim_site_Kid_");
    assert_eq!(sites.len(), 1, "pending: work reclaims its Kid through one site call:\n{work}");
    assert_eq!(assert_pending_guarded("pending", function(&ir, sites[0])), 1, "pending: `{}` checks Kid's claim", sites[0]);
    assert!(assert_pending_guarded("pending", function(&ir, "__reclaim_Kid")) >= 1, "pending: __reclaim_Kid checks its claim");

    let ir = ir_of("pending_live", "locus Box {\n    params { k: Kid = Kid { tag: 1 }; }\n}\n\nfn main() { Box { }; }\n");
    let main = function(&ir, "main");
    assert!(!main.contains("@lotus_reclaim_pending("), "pending_live: main checks no claim itself:\n{main}");
    for l in ["Box", "Kid"] {
        let live = format!("__reclaim_live_{l}");
        assert!(!calls_of(main, &live).is_empty(), "pending_live: main skips a reclaimed {l} through `{live}`:\n{main}");
        assert_eq!(assert_pending_guarded("pending_live", function(&ir, &live)), 1, "pending_live: `{live}` checks the claim");
    }
}

/// An owner's cascade enters its reclaim scope through the shared
/// `__reclaim_scope_enter`, whose call is behind the quiet guard, and
/// leaves it through `__reclaim_scope_leave`, whose call is behind the
/// scope's own null test.
#[test]
fn the_reclaim_scope_is_entered_and_left_behind_its_guards() {
    let ir = ir_of("scope", "locus Box {\n    params { k: Kid = Kid { tag: 1 }; }\n}\n\nfn main() { Box { }; }\n");
    for name in ["__reclaim_Box", "main"] {
        let f = function(&ir, name);
        assert_eq!(calls_of(f, "__reclaim_scope_enter").len(), 1, "scope: `{name}` enters Box's cascade scope once:\n{f}");
        assert_eq!(calls_of(f, "__reclaim_scope_leave").len(), 1, "scope: `{name}` leaves it once:\n{f}");
        assert!(!f.contains("@lotus_reclaim_scope_enter(") && !f.contains("@lotus_reclaim_scope_leave("),
            "scope: `{name}` calls the runtime's scope itself:\n{f}");
    }
    assert_guarded("scope", function(&ir, "__reclaim_scope_enter"), "lotus_reclaim_scope_enter", QUIET_WORDS);
    let leave = function(&ir, "__reclaim_scope_leave");
    let left = blocks(leave).into_iter().filter(|b| b.body.contains("@lotus_reclaim_scope_leave(")).collect::<Vec<_>>();
    assert_eq!(left.len(), 1, "scope: one leave call:\n{leave}");
    for b in left {
        assert!(b.label.starts_with("reclaim.scope.leave.call"), "scope: the leave in `{}` is unguarded", b.label);
        let br = branch_to(leave, b.label).expect("the leave's guard");
        let cond = br["br i1 %".len()..].split(',').next().expect("condition");
        assert_eq!(definition(leave, cond), "icmp eq ptr %0, null", "scope: the leave's guard tests its scope: {br}");
    }
}

/// A cascade that tears no field down runs nothing a scope could
/// collect, and enters none; the let-bound and the spine reclaim of a
/// fieldless locus make no scope call at all.
#[test]
fn a_cascade_that_tears_no_field_down_enters_no_scope() {
    let ir = ir_of("no_scope", "fn work() {\n    let k = Kid { tag: 1 };\n    println(\"ev v \" + to_string(k.v()));\n}\n\nfn main() { work(); }\n");
    let mut names = vec!["work", "__reclaim_Kid"];
    names.extend(calls_of(function(&ir, "work"), "__reclaim_site_Kid_"));
    for name in names {
        let f = function(&ir, name);
        for scope in ["@lotus_reclaim_scope_enter(", "@lotus_reclaim_scope_leave(", "@__reclaim_scope_enter(", "@__reclaim_scope_leave("] {
            assert!(!f.contains(scope), "no_scope: `{name}` reclaims a Kid, which has no field to tear down:\n{f}");
        }
    }
}

/// The dissolve cascade: an owner's field is reclaimed inside the
/// owner's teardown, before the owner's own arena.
#[test]
fn dissolve_cascade_cancels_before_the_arena_goes() {
    let ir = ir_of("cascade", "locus Box {\n    params { k: Kid = Kid { tag: 1 }; }\n}\n\nfn main() { Box { }; }\n");
    assert_spine("cascade", &ir, "main", "Kid");
    assert_spine("cascade", &ir, "main", "Box");
}
