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
