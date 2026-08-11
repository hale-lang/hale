//! Recognized element chains (#353 cluster B).
//!
//! `xs.filter(it > 2).count()` is not a value being built — it is a
//! form the compiler recognizes and rewrites to ONE loop.
//!
//! The knot that made "add closures, then add iterators" look
//! expensive was self-inflicted: it assumed each stage must PRODUCE
//! something, so a chain needs either a lazy object or an intermediate
//! collection, either of which is a sequence value needing an owner,
//! which reopens arenas and placement. If the chain is a recognized
//! form rather than a value, nothing is produced at any step and the
//! question never arises.
//!
//! Three consequences, all of which fall out rather than being
//! engineered, and all pinned below:
//!
//!   - ZERO ALLOCATION, so a chain is legal inside
//!     `@budget(alloc_per_call = 0)`. A design returning a new
//!     collection would be illegal in exactly the code Hale exists
//!     for.
//!   - EAGER, so a predicate's effects are attributed to the
//!     predicate's own source position. A lazy chain would run it at
//!     the terminal and the witness path would name the wrong line.
//!   - NO LAMBDAS. The predicate is an argument position the compiler
//!     knows about, not a value, so there is no closure to represent —
//!     no capture modes, no escape analysis, no cross-thread question.
//!     `it` is bound per element by the desugar.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!(
        "hale_chain_{}_{}",
        name,
        std::process::id()
    ));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

const V: &str =
    "@form(vec)\nlocus Nums { capacity { heap items of Int; } }\n";

#[test]
fn filter_count_evaluates_the_predicate_per_element() {
    let src = format!(
        "{V}fn main() {{
            let v = Nums {{ }};
            v.push(1); v.push(5); v.push(2); v.push(9);
            println(\"gt2=\", v.filter(it > 2).count());
            println(\"all=\", v.filter(it > 0).count());
            println(\"none=\", v.filter(it > 100).count());
        }}"
    );
    let (out, st) = run("count", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("gt2=2"), "5 and 9 pass: {:?}", out);
    assert!(out.contains("all=4"), "got: {:?}", out);
    assert!(out.contains("none=0"), "got: {:?}", out);
}

/// Two stages fuse into ONE pass — no intermediate exists for the
/// second stage to read.
#[test]
fn two_stages_fuse() {
    let src = format!(
        "{V}fn main() {{
            let v = Nums {{ }};
            v.push(1); v.push(5); v.push(2); v.push(9);
            println(\"fused=\", v.filter(it > 2).filter(it < 9).count());
        }}"
    );
    let (out, st) = run("fuse", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("fused=1"), "only 5 survives both: {:?}", out);
}

/// THE claim. A chain allocates nothing, so it is legal on a hot path.
/// If this ever fails, the design has regressed to materialising
/// something, and composition stops being usable in the code Hale is
/// built for.
#[test]
fn a_chain_is_legal_under_a_zero_alloc_budget() {
    let src = format!(
        "{V}@budget(alloc_per_call = 0)
        fn big(v: Nums) -> Int {{
            return v.filter(it > 2).filter(it < 9).count();
        }}
        fn main() {{
            let v = Nums {{ }};
            v.push(1); v.push(5); v.push(9);
            println(\"n=\", big(v));
        }}"
    );
    let (out, st) = run("budget", &src);
    assert!(
        st.success(),
        "a chain must not allocate — @budget(alloc_per_call = 0) \
         rejected it: {:?}",
        st
    );
    assert!(out.contains("n=1"), "got: {:?}", out);
}

/// The `into` terminal writes into caller-supplied storage — the same
/// shape `split_into` uses, and the only point at which anything is
/// materialised.
#[test]
fn into_writes_the_survivors_to_a_target() {
    let src = format!(
        "{V}fn main() {{
            let v = Nums {{ }};
            v.push(1); v.push(5); v.push(2); v.push(9);
            let out = Nums {{ }};
            v.filter(it > 2).into(out);
            println(\"n=\", out.len(), \" first=\", out.get(0) or -1);
        }}"
    );
    let (out, st) = run("into", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("n=2"), "5 and 9: {:?}", out);
    assert!(out.contains("first=5"), "in source order: {:?}", out);
}

/// Two chains in one block must not collide on their loop
/// temporaries — the desugar numbers them.
#[test]
fn two_chains_in_one_block_coexist() {
    let src = format!(
        "{V}fn main() {{
            let v = Nums {{ }};
            v.push(1); v.push(5);
            let a = v.filter(it > 0).count();
            let b = v.filter(it > 4).count();
            println(\"a=\", a, \" b=\", b);
        }}"
    );
    let (out, st) = run("two", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("a=2"), "got: {:?}", out);
    assert!(out.contains("b=1"), "got: {:?}", out);
}

/// A bare `.count()` with no stage is an ordinary form method, not a
/// chain. The desugar must not swallow it.
#[test]
fn a_bare_method_is_not_a_chain() {
    let src = format!(
        "{V}fn main() {{
            let v = Nums {{ }};
            v.push(1); v.push(2);
            println(\"len=\", v.len());
        }}"
    );
    let (out, st) = run("bare", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("len=2"), "got: {:?}", out);
}

// ==== 2026-08-04 vocabulary tranche ================================
//
// map / sum / any / all / first / find / min / max / each. Same
// mechanism, more rows in the table. The struct-element source
// exercises `it`-substitution through field access; the fallible
// terminals ride the source's own `get` (IndexError + ordinary
// `or`), which is also why they refuse to compose with `map`.

const U: &str = r#"
    type User { id: Int; age: Int; name: String; }

    @form(vec)
    locus Users { capacity { heap data of User; } }

    @form(vec)
    locus Ints { capacity { heap data of Int; } }

    fn seed(us: Users) {
        us.push(User { id: 1, age: 30, name: "ann" });
        us.push(User { id: 2, age: 17, name: "bob" });
        us.push(User { id: 3, age: 45, name: "cid" });
        us.push(User { id: 4, age: 22, name: "dee" });
    }
"#;

#[test]
fn map_sum_and_stage_composition() {
    let src = format!(
        "{U}fn main() {{
            let us = Users {{ }};
            seed(us);
            println(\"s=\", us.map(it.age).sum());
            println(\"a=\", us.filter(it.age >= 18).map(it.age).sum());
            println(\"c=\", us.filter(it.age >= 18).count());
        }}"
    );
    let (out, st) = run("map_sum", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("s=114"), "got: {:?}", out);
    assert!(out.contains("a=97"), "got: {:?}", out);
    assert!(out.contains("c=3"), "got: {:?}", out);
}

#[test]
fn any_all_including_vacuous_truth() {
    let src = format!(
        "{U}fn main() {{
            let us = Users {{ }};
            seed(us);
            println(\"minor=\", us.any(it.age < 18));
            println(\"all_adult=\", us.all(it.age >= 18));
            println(\"all_named=\", us.all(len(it.name) > 0));
            // vacuous cases on an emptied selection
            println(\"v_any=\", us.filter(it.age > 100).any(true));
            println(\"v_all=\", us.filter(it.age > 100).all(false));
        }}"
    );
    let (out, st) = run("any_all", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("minor=true"), "got: {:?}", out);
    assert!(out.contains("all_adult=false"), "got: {:?}", out);
    assert!(out.contains("all_named=true"), "got: {:?}", out);
    // any over nothing is false; all over nothing is true — spec'd
    // vacuous truth, pinned here so nobody "fixes" it.
    assert!(out.contains("v_any=false"), "got: {:?}", out);
    assert!(out.contains("v_all=true"), "got: {:?}", out);
}

#[test]
fn find_first_min_max_are_fallible_on_empty() {
    let src = format!(
        "{U}fn main() {{
            let us = Users {{ }};
            seed(us);
            let bob = us.find(it.id == 2) or User {{ id: 0, age: 0, name: \"?\" }};
            let ghost = us.find(it.id == 99) or User {{ id: 0, age: 0, name: \"ghost\" }};
            let fm = us.filter(it.age < 18).first() or User {{ id: 0, age: 0, name: \"none\" }};
            let young = us.min(it.age) or User {{ id: 0, age: 0, name: \"?\" }};
            let old = us.max(it.age) or User {{ id: 0, age: 0, name: \"?\" }};
            let none = us.filter(it.age > 100).min(it.age) or User {{ id: 0, age: 0, name: \"empty\" }};
            println(\"bob=\", bob.name);
            println(\"ghost=\", ghost.name);
            println(\"fm=\", fm.name);
            println(\"young=\", young.name);
            println(\"old=\", old.name);
            println(\"none=\", none.name);
        }}"
    );
    let (out, st) = run("find_min_max", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("bob=bob"), "got: {:?}", out);
    assert!(out.contains("ghost=ghost"), "got: {:?}", out);
    assert!(out.contains("fm=bob"), "got: {:?}", out);
    assert!(out.contains("young=bob"), "got: {:?}", out);
    assert!(out.contains("old=cid"), "got: {:?}", out);
    assert!(out.contains("none=empty"), "got: {:?}", out);
}

#[test]
fn map_into_lands_mapped_elements() {
    let src = format!(
        "{U}fn main() {{
            let us = Users {{ }};
            seed(us);
            let ages = Ints {{ }};
            us.filter(it.age >= 18).map(it.age).into(ages);
            println(\"n=\", ages.len());
            println(\"s=\", ages.filter(true).sum());
        }}"
    );
    let (out, st) = run("map_into", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("n=3"), "got: {:?}", out);
    assert!(out.contains("s=97"), "got: {:?}", out);
}

/// `each { ... }` is the fused loop's body: side effects run per
/// surviving element, and `continue` / `break` act on the loop —
/// `continue` must advance to the NEXT element (the increment-first
/// loop shape; an end-of-body increment would spin forever).
#[test]
fn each_block_with_continue_and_break() {
    let src = format!(
        "{U}fn main() {{
            let us = Users {{ }};
            seed(us);
            let mut total = 0;
            us.filter(it.age >= 18).each {{
                total = total + it.age;
                println(\"visit=\", it.name);
            }}
            println(\"total=\", total);
            let mut seen = 0;
            us.each {{
                if it.id == 2 {{ continue; }}
                if it.id == 3 {{ break; }}
                seen = seen + it.age;
            }}
            println(\"seen=\", seen);
        }}"
    );
    let (out, st) = run("each", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("visit=ann"), "got: {:?}", out);
    assert!(out.contains("visit=cid"), "got: {:?}", out);
    assert!(out.contains("visit=dee"), "got: {:?}", out);
    assert!(out.contains("total=97"), "got: {:?}", out);
    assert!(out.contains("seen=30"), "got: {:?}", out);
}

/// User facade methods that share terminal names keep resolving:
/// stage-less calls whose arguments do not mention `it` are ordinary
/// method calls, never chains. The hijack-safety half of the
/// recognition gate.
#[test]
fn user_methods_named_like_terminals_still_resolve() {
    let src = r#"
        locus Counterish {
            params { hits: Int = 0; }
            fn any(v: Int) -> Bool { self.hits = self.hits + 1; return v > 0; }
            fn find(v: Int) -> Int { return v * 2; }
        }
        fn main() {
            let c = Counterish { };
            println("a=", c.any(5));
            println("f=", c.find(21));
        }
    "#;
    let (out, st) = run("facade", src);
    assert!(st.success(), "non-zero: {:?}", st);
    assert!(out.contains("a=true"), "got: {:?}", out);
    assert!(out.contains("f=42"), "got: {:?}", out);
}

/// A chain nested inside another chain's predicate.
#[test]
fn nested_chain_in_a_predicate() {
    let src = format!(
        "{U}fn main() {{
            let us = Users {{ }};
            seed(us);
            let above = us.filter(it.age > us.map(it.age).sum() / 4).count();
            println(\"above=\", above);
        }}"
    );
    let (out, st) = run("nested", &src);
    assert!(st.success(), "non-zero: {:?}", st);
    // avg = 114/4 = 28 -> ages 30, 45 above.
    assert!(out.contains("above=2"), "got: {:?}", out);
}

/// A find miss under `or raise` diverges like any raised fallible —
/// the process exits non-zero rather than fabricating an element.
#[test]
fn find_miss_or_raise_diverges() {
    let src = format!(
        "{U}fn main() {{
            let us = Users {{ }};
            seed(us);
            let hit = us.find(it.id == 2) or raise;
            println(\"hit=\", hit.name);
            let gone = us.find(it.id == 99) or raise;
            println(\"unreachable=\", gone.name);
        }}"
    );
    let (out, st) = run("find_raise", &src);
    assert!(
        !st.success(),
        "a raised find miss must not exit clean: {:?}\n{}",
        st,
        out
    );
    assert!(out.contains("hit=bob"), "the hit path runs first: {:?}", out);
    assert!(
        !out.contains("unreachable="),
        "the miss must diverge: {:?}",
        out
    );
}

// ==== 2026-08-11 tranche: take / skip / enumerate / seeded sum /
// ==== sort_into / reverse_into / group_count_into.
//
// The vocabulary is combinatorial — 5 orderable stages x 6 terminal
// families x 2 sources x 3 contexts — so coverage here is pairwise
// over the interaction classes rather than cartesian: every NEW
// stage meets every terminal FAMILY at least once, both orders of
// every order-sensitive stage pair are pinned, and the dangerous
// cells (early-break interactions, `idx` reach, counter re-init on
// re-execution, uid separation) get their own probes.

const F: &str =
    "@form(vec)\nlocus Floats { capacity { heap items of Float; } }\n";

const T: &str = "type Tally { tag: String; n: Int; }\n\
    @form(hashmap)\n\
    locus Tallies { capacity { pool entries of Tally indexed_by tag; } }\n";

fn seed_ten() -> &'static str {
    "let v = Nums { };\n\
     let mut i = 1;\n\
     while i <= 10 { v.push(i); i = i + 1; }\n"
}

/// take / skip count elements ARRIVING at their own stage position,
/// and the two orders of skip x take are different chains: 1..10 has
/// skip(1).take(3) = {2,3,4} while take(3).skip(1) = {2,3}.
#[test]
fn skip_take_both_orders_and_filter_position() {
    let src = format!(
        "{V}fn main() {{
            {seed}
            println(\"st=\", v.skip(1).take(3).sum());
            println(\"ts=\", v.take(3).skip(1).sum());
            println(\"ft=\", v.filter(it % 2 == 0).take(2).sum());
            println(\"tf=\", v.take(2).filter(it % 2 == 0).sum());
            println(\"fs=\", v.filter(it % 2 == 0).skip(3).sum());
            println(\"sf=\", v.skip(3).filter(it % 2 == 0).sum());
        }}",
        seed = seed_ten()
    );
    let (out, st) = run("skip_take_orders", &src);
    assert!(st.success(), "non-zero: {:?}\n{}", st, out);
    assert!(out.contains("st=9"), "2+3+4: {:?}", out);
    assert!(out.contains("ts=5"), "2+3: {:?}", out);
    assert!(out.contains("ft=6"), "2+4 (first two matches): {:?}", out);
    assert!(out.contains("tf=2"), "even of {{1,2}}: {:?}", out);
    assert!(out.contains("fs=18"), "8+10 (skip 3 MATCHES): {:?}", out);
    assert!(out.contains("sf=28"), "4+6+8+10: {:?}", out);
}

/// take(0) never admits an element; saturation and repeated
/// positional stages compose; limits are ordinary expressions.
#[test]
fn take_zero_saturation_and_stacked_positionals() {
    let src = format!(
        "{V}fn main() {{
            {seed}
            println(\"t0=\", v.take(0).count());
            println(\"big=\", v.take(99).count());
            println(\"tt=\", v.take(5).take(2).sum());
            println(\"ss=\", v.skip(4).skip(4).sum());
            let n = 2;
            println(\"expr=\", v.take(n + 1).sum());
        }}",
        seed = seed_ten()
    );
    let (out, st) = run("take_zero", &src);
    assert!(st.success(), "non-zero: {:?}\n{}", st, out);
    assert!(out.contains("t0=0"), "got: {:?}", out);
    assert!(out.contains("big=10"), "limit past the end: {:?}", out);
    assert!(out.contains("tt=3"), "1+2 (inner take wins): {:?}", out);
    assert!(out.contains("ss=19"), "9+10: {:?}", out);
    assert!(out.contains("expr=6"), "1+2+3: {:?}", out);
}

/// `idx` counts the stream reaching the enumerate STAGE: before a
/// filter it is the source index, after it is the match ordinal. A
/// second enumerate shadows the first for everything after it.
#[test]
fn enumerate_position_and_shadowing() {
    let src = format!(
        "{V}fn main() {{
            {seed}
            println(\"pre=\", v.enumerate().filter(idx % 2 == 0).sum());
            println(\"post=\", v.filter(it > 5).enumerate().map(idx).sum());
            println(\"both=\", v.enumerate().map(it + idx).sum());
            println(\"shadow=\", v.enumerate().filter(idx >= 2).enumerate().map(idx).sum());
        }}",
        seed = seed_ten()
    );
    let (out, st) = run("enumerate_pos", &src);
    assert!(st.success(), "non-zero: {:?}\n{}", st, out);
    assert!(out.contains("pre=25"), "1+3+5+7+9 (source idx): {:?}", out);
    assert!(out.contains("post=10"), "0+1+2+3+4 (match ordinal): {:?}", out);
    assert!(out.contains("both=100"), "55 + 45: {:?}", out);
    // outer idx filters to elements 3..10 (8 of them); inner idx
    // renumbers them 0..7 -> sum 28.
    assert!(out.contains("shadow=28"), "inner enumerate renumbers: {:?}", out);
}

/// Every new stage against the indexed-fallible family — the stage
/// counters live in `lower_indexed`'s init path, and the take break
/// must leave the best-so-far index intact.
#[test]
fn positional_stages_with_indexed_terminals() {
    let src = format!(
        "{V}fn main() {{
            {seed}
            println(\"sf=\", v.filter(it % 2 == 0).skip(2).first() or -1);
            println(\"tm=\", v.take(4).max() or -1);
            println(\"ef=\", v.enumerate().filter(idx == it - 1).first() or -1);
            println(\"miss=\", v.skip(10).first() or -1);
            println(\"t0m=\", v.take(0).min() or -1);
            println(\"sfind=\", v.skip(2).find(it % 3 == 0) or -1);
        }}",
        seed = seed_ten()
    );
    let (out, st) = run("positional_indexed", &src);
    assert!(st.success(), "non-zero: {:?}\n{}", st, out);
    assert!(out.contains("sf=6"), "third even: {:?}", out);
    assert!(out.contains("tm=4"), "max of first four: {:?}", out);
    assert!(out.contains("ef=1"), "idx==it-1 holds from the start: {:?}", out);
    assert!(out.contains("miss=-1"), "skip past the end is empty: {:?}", out);
    assert!(out.contains("t0m=-1"), "take(0) min is empty: {:?}", out);
    assert!(out.contains("sfind=3"), "find after skip sees 3 first: {:?}", out);
}

/// Early-break interactions: any's break vs take's break, all's
/// vacuous truth over a take(0) selection, and a `continue` inside
/// an each block advancing PAST an already-counted element.
#[test]
fn early_break_interactions() {
    let src = format!(
        "{V}fn main() {{
            {seed}
            println(\"a=\", v.take(3).any(it > 2));
            println(\"a2=\", v.take(2).any(it > 5));
            println(\"vac=\", v.take(0).all(it > 100));
            let mut seen = 0;
            v.take(4).each {{
                if it % 2 == 0 {{ continue; }}
                seen = seen + it;
            }}
            println(\"seen=\", seen);
            let mut idxsum = 0;
            v.filter(it > 6).enumerate().each {{ idxsum = idxsum + idx; }}
            println(\"idxsum=\", idxsum);
        }}",
        seed = seed_ten()
    );
    let (out, st) = run("early_break", &src);
    assert!(st.success(), "non-zero: {:?}\n{}", st, out);
    assert!(out.contains("a=true"), "3 > 2 within take: {:?}", out);
    assert!(out.contains("a2=false"), "take caps before a hit: {:?}", out);
    assert!(out.contains("vac=true"), "empty all is true: {:?}", out);
    assert!(out.contains("seen=4"), "1+3, continue skips evens: {:?}", out);
    assert!(out.contains("idxsum=6"), "0+1+2+3 over four matches: {:?}", out);
}

/// sum(seed): the seed is the accumulator's typed zero (Float) and
/// its starting value (Int offset); a map stage composes.
#[test]
fn seeded_sum_float_and_offset() {
    let src = format!(
        "{V}{F}fn main() {{
            let f = Floats {{ }};
            f.push(1.5); f.push(2.25); f.push(3.25);
            println(\"f=\", f.filter(it > 1.6).sum(0.0));
            println(\"fm=\", f.map(it + 0.5).sum(0.0));
            {seed}
            println(\"off=\", v.take(2).sum(100));
            println(\"empty=\", v.take(0).sum(7));
        }}",
        seed = seed_ten()
    );
    let (out, st) = run("seeded_sum", &src);
    assert!(st.success(), "non-zero: {:?}\n{}", st, out);
    assert!(out.contains("f=5.5"), "2.25+3.25: {:?}", out);
    assert!(out.contains("fm=8.5"), "2.0+2.75+3.75: {:?}", out);
    assert!(out.contains("off=103"), "100+1+2: {:?}", out);
    assert!(out.contains("empty=7"), "empty selection = seed: {:?}", out);
}

/// The whole-set terminals materialize into caller storage and then
/// reorder it: sort_into (primitive + comparator), reverse_into, and
/// take saturation must still run the post-loop reorder.
#[test]
fn sort_and_reverse_into() {
    let src = format!(
        "{V}fn desc(a: Int, b: Int) -> Bool {{ return a > b; }}
        fn main() {{
            let v = Nums {{ }};
            v.push(3); v.push(9); v.push(1); v.push(7); v.push(5);
            let s = Nums {{ }};
            v.filter(it > 1).sort_into(s);
            println(\"s=\", s.get(0) or -1, \",\", s.get(3) or -1, \",\", s.len());
            let d = Nums {{ }};
            v.sort_into(d, desc);
            println(\"d=\", d.get(0) or -1, \",\", d.get(4) or -1);
            let r = Nums {{ }};
            v.take(3).reverse_into(r);
            println(\"r=\", r.get(0) or -1, \",\", r.get(1) or -1, \",\", r.get(2) or -1);
            let ts = Nums {{ }};
            v.take(2).sort_into(ts);
            println(\"ts=\", ts.get(0) or -1, \",\", ts.get(1) or -1);
        }}"
    );
    let (out, st) = run("sort_reverse", &src);
    assert!(st.success(), "non-zero: {:?}\n{}", st, out);
    assert!(out.contains("s=3,9,4"), "sorted survivors: {:?}", out);
    assert!(out.contains("d=9,1"), "comparator descends: {:?}", out);
    assert!(out.contains("r=1,9,3"), "first three reversed: {:?}", out);
    assert!(out.contains("ts=3,9"), "post-break reorder still runs: {:?}", out);
}

/// group_count_into rides the hashmap's bump: keyed by an
/// it-expression, by the element itself (bare), by `idx` through an
/// enumerate, and ACCUMULATING across two chains (increment-or-init).
#[test]
fn group_count_into_tallies() {
    let src = format!(
        "{V}{T}fn main() {{
            {seed}
            let t = Tallies {{ }};
            v.group_count_into(t, if it % 2 == 0 {{ \"even\" }} else {{ \"odd\" }});
            let e = t.get(\"even\") or Tally {{ tag: \"?\", n: -1 }};
            let o = t.get(\"odd\") or Tally {{ tag: \"?\", n: -1 }};
            println(\"eo=\", e.n, \",\", o.n);
            v.take(4).group_count_into(t, \"even\");
            let e2 = t.get(\"even\") or Tally {{ tag: \"?\", n: -1 }};
            println(\"acc=\", e2.n);
            let b = Tallies {{ }};
            v.map(if it > 5 {{ \"hi\" }} else {{ \"lo\" }}).group_count_into(b, it);
            let hi = b.get(\"hi\") or Tally {{ tag: \"?\", n: -1 }};
            println(\"bare=\", hi.n);
            let g = Tallies {{ }};
            v.enumerate().group_count_into(g, if idx < 3 {{ \"head\" }} else {{ \"tail\" }});
            let tl = g.get(\"tail\") or Tally {{ tag: \"?\", n: -1 }};
            println(\"idxkey=\", tl.n);
        }}",
        seed = seed_ten()
    );
    let (out, st) = run("group_count", &src);
    assert!(st.success(), "non-zero: {:?}\n{}", st, out);
    assert!(out.contains("eo=5,5"), "five even, five odd: {:?}", out);
    assert!(out.contains("acc=9"), "5 + 4 more bumps accumulate: {:?}", out);
    assert!(out.contains("bare=5"), "mapped element as key: {:?}", out);
    assert!(out.contains("idxkey=7"), "idx reaches the key: {:?}", out);
}

/// The hashmap `.entries` source drives the same counter machinery
/// through `entry_at`, including regrouping one map into another.
#[test]
fn hashmap_source_with_new_stages() {
    let src = format!(
        "{T}fn main() {{
            let t = Tallies {{ }};
            t.set(Tally {{ tag: \"a\", n: 3 }});
            t.set(Tally {{ tag: \"b\", n: 5 }});
            t.set(Tally {{ tag: \"c\", n: 7 }});
            t.set(Tally {{ tag: \"d\", n: 9 }});
            println(\"take=\", t.entries.take(2).count());
            println(\"skip=\", t.entries.skip(3).count());
            println(\"sum=\", t.entries.map(it.n).sum());
            println(\"enum=\", t.entries.enumerate().map(idx).sum());
            let g = Tallies {{ }};
            t.entries.filter(it.n > 4).group_count_into(g, \"big\");
            let big = g.get(\"big\") or Tally {{ tag: \"?\", n: -1 }};
            println(\"regroup=\", big.n);
        }}"
    );
    let (out, st) = run("hashmap_stages", &src);
    assert!(st.success(), "non-zero: {:?}\n{}", st, out);
    assert!(out.contains("take=2"), "got: {:?}", out);
    assert!(out.contains("skip=1"), "got: {:?}", out);
    assert!(out.contains("sum=24"), "3+5+7+9: {:?}", out);
    assert!(out.contains("enum=6"), "0+1+2+3: {:?}", out);
    assert!(out.contains("regroup=3"), "5, 7 and 9 pass n>4: {:?}", out);
}

/// Context axes: a chain re-executed in a loop re-inits its
/// counters; two chains with the same stages in one block keep
/// their uids apart; a chain nested in a stage argument carries its
/// own counters; statement-position sort_into splices with its
/// post statements intact.
#[test]
fn counter_reinit_uid_separation_and_nesting() {
    let src = format!(
        "{V}fn main() {{
            {seed}
            let mut round = 0;
            let mut total = 0;
            while round < 3 {{
                total = total + v.skip(8).take(1).sum();
                round = round + 1;
            }}
            println(\"reinit=\", total);
            let a = v.take(2).sum();
            let b = v.take(3).sum();
            println(\"uids=\", a, \",\", b);
            println(\"nested=\", v.filter(it > v.take(4).sum() - 7).count());
            let s = Nums {{ }};
            v.skip(7).sort_into(s);
            println(\"stmt=\", s.get(0) or -1, \",\", s.len());
        }}",
        seed = seed_ten()
    );
    let (out, st) = run("reinit_uid", &src);
    assert!(st.success(), "non-zero: {:?}\n{}", st, out);
    assert!(out.contains("reinit=27"), "9 three times, fresh counters: {:?}", out);
    assert!(out.contains("uids=3,6"), "1+2 and 1+2+3: {:?}", out);
    // take(4).sum() = 10; filter(it > 3) -> 7 elements.
    assert!(out.contains("nested=7"), "inner chain in a predicate: {:?}", out);
    assert!(out.contains("stmt=8,3"), "statement-position post ran: {:?}", out);
}

/// Method context: the desugar walks locus fn bodies; a self-field
/// source and a self-field target work with the new stages.
#[test]
fn method_context_new_stages() {
    let src = format!(
        "{V}locus Holder {{
            params {{ xs: Nums = Nums {{ }}; out: Nums = Nums {{ }}; }}
            fn fill() {{
                let mut i = 1;
                while i <= 6 {{ self.xs.push(i); i = i + 1; }}
            }}
            fn head_sum(n: Int) -> Int {{
                return self.xs.take(n).sum();
            }}
            fn tail_sorted_desc() {{
                self.xs.skip(4).sort_into(self.out, __desc);
            }}
        }}
        fn __desc(a: Int, b: Int) -> Bool {{ return a > b; }}
        fn main() {{
            let h = Holder {{ }};
            h.fill();
            println(\"hs=\", h.head_sum(3));
            h.tail_sorted_desc();
            println(\"ts=\", h.out.get(0) or -1, \",\", h.out.get(1) or -1);
        }}"
    );
    let (out, st) = run("method_ctx", &src);
    assert!(st.success(), "non-zero: {:?}\n{}", st, out);
    assert!(out.contains("hs=6"), "1+2+3: {:?}", out);
    assert!(out.contains("ts=6,5"), "5,6 sorted descending: {:?}", out);
}

/// Negative space. A min/max key that mentions `idx` is NOT lowered
/// (the best's enumerate count is unrecoverable at compare time) —
/// it must fail to build, never miscompare.
#[test]
fn min_key_mentioning_idx_is_rejected() {
    let src = format!(
        "{V}fn main() {{
            let v = Nums {{ }};
            v.push(3);
            let m = v.enumerate().min(it + idx) or -1;
            println(\"m=\", m);
        }}"
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let bin = harness::unique_bin(&format!(
        "hale_chain_minidx_{}",
        std::process::id()
    ));
    let res = build_executable(&program, &bin);
    let _ = std::fs::remove_file(&bin);
    assert!(res.is_err(), "an idx-keyed min must not lower");
}

/// Negative space, facade half: user methods named like the new
/// stages resolve as ordinary calls when no chain terminal follows,
/// and a stage-less seeded sum / bare-keyed group_count_into stay
/// ordinary calls (conservative recognition, unchanged).
#[test]
fn new_stage_names_do_not_hijack_facades() {
    let src = r#"
        locus Pager {
            params { width: Int = 10; }
            fn take(n: Int) -> Int { return n * self.width; }
            fn skip(n: Int) -> Int { return n + self.width; }
        }
        fn main() {
            let p = Pager { };
            println("t=", p.take(3));
            println("s=", p.skip(3));
        }
    "#;
    let (out, st) = run("facade_take", src);
    assert!(st.success(), "non-zero: {:?}\n{}", st, out);
    assert!(out.contains("t=30"), "user take resolves: {:?}", out);
    assert!(out.contains("s=13"), "user skip resolves: {:?}", out);
}

/// Name threading: `map` rebinds the element to a fresh local, and
/// take/skip/enumerate pass the CURRENT name through — a map on each
/// side of a positional stage must see its own element.
#[test]
fn map_sandwich_around_positional_stages() {
    let src = format!(
        "{V}fn main() {{
            {seed}
            let t = Nums {{ }};
            v.map(it * 2).take(3).map(it + 1).into(t);
            println(\"t=\", t.get(0) or -1, \",\", t.get(2) or -1, \",\", t.len());
            println(\"s=\", v.map(it * 10).skip(8).enumerate().map(it + idx).sum());
        }}",
        seed = seed_ten()
    );
    let (out, st) = run("map_sandwich", &src);
    assert!(st.success(), "non-zero: {:?}\n{}", st, out);
    assert!(out.contains("t=3,7,3"), "(1,2,3)*2+1 = 3,5,7: {:?}", out);
    // 90+0 and 100+1 -> 191.
    assert!(out.contains("s=191"), "maps on both sides of skip/enumerate: {:?}", out);
}
