//! GH #2134 (F.63): what `hale check` refuses in a `@vectorize` fn,
//! because no target's vectorizer takes it. Each refusal is placed at
//! the thing that stops the loop and says what it is; what is left is
//! the build's target-dependent assertion (hale-codegen's
//! `vectorize_assertion.rs`).

#[path = "support/entries.rs"]
mod entries;
use entries::check_program;
use hale_syntax::parse_source;

/// The `@vectorize` refusals of `src`, each as `line:col message`.
fn refusals(src: &str) -> Vec<String> {
    let prog = parse_source(src).expect("parse");
    check_program(&prog)
        .into_iter()
        .filter(|d| d.message.starts_with("`@vectorize` on "))
        .map(|d| {
            let (l, c) = d.span.line_col(src);
            format!("{l}:{c} {}", d.message)
        })
        .collect()
}

fn one(src: &str, at: &str, says: &str) {
    let r = refusals(src);
    assert_eq!(r.len(), 1, "one refusal: {r:#?}");
    assert!(r[0].starts_with(at), "at {at}: {r:#?}");
    assert!(r[0].contains(says), "says {says:?}: {r:#?}");
    assert!(r[0].contains("this loop cannot vectorize on any target"), "{r:#?}");
}

#[test]
fn a_kernel_that_can_vectorize_is_clean() {
    let src = "fn twice(x: Int) -> Int { return x * 2; }
@vectorize(min_width: 2)
fn kernel(xs: bounded[Int; 64]) -> Int {
    let mut s = 0;
    for x in xs {
        s = s + twice(x);
    }
    return s + xs.map(it * 3).sum(0);
}
locus L {
    params { k: Int = 2; }
    @vectorize
    fn scaled(xs: [Int; 8]) -> Int {
        let mut s = 0;
        for i in 0..8 {
            s = s + xs[i] * self.k;
        }
        return s;
    }
}
fn main() { let xs: bounded[Int; 64] = []; let l = L { }; println(kernel(xs) + l.scaled([1; 8])); }
";
    assert_eq!(refusals(src), Vec::<String>::new());
}

#[test]
fn an_early_return_is_a_second_exit() {
    one(
        "@vectorize
fn f(xs: [Int; 8]) -> Int {
    let mut s = 0;
    for x in xs {
        if x < 0 { return s; }
        s = s + x;
    }
    return s;
}
fn main() { println(f([1; 8])); }
",
        "5:20",
        "a second way out",
    );
}

#[test]
fn a_break_is_a_second_exit_but_a_chains_own_exit_is_not() {
    one(
        "@vectorize
fn f(xs: [Int; 8]) -> Int {
    let mut s = 0;
    for x in xs {
        if x < 0 { break; }
        s = s + x;
    }
    return s + xs.map(it).sum(0);
}
fn main() { println(f([1; 8])); }
",
        "5:20",
        "a second way out",
    );
}

/// What the vectorizer may still take is the build's to judge, not the
/// check's: a fallible step whose failure branch bounds-check elimination
/// removes, a text value read in the loop and hoisted out of it, a
/// `while true` whose one `break` is its exit, a call through a fn value.
#[test]
fn what_may_still_vectorize_is_left_to_the_build() {
    let src = "@form(vec)
locus V { capacity { heap items of Int; } }
fn double(x: Int) -> Int { return x * 2; }
@vectorize
fn f(v: V, name: String, g: fn(Int) -> Int) -> Int {
    let mut acc = 0;
    for j in 0..v.len() {
        acc = acc ^ (v.get(j) or 0);
    }
    for j in 0..v.len() {
        acc = acc + len(name) + g(j);
    }
    let mut i = 0;
    while true {
        if i >= 8 { break; }
        acc = acc + i;
        i = i + 1;
    }
    return acc;
}
fn main() { let v = V { }; v.push(1); println(f(v, \"n\", double)); }
";
    assert_eq!(refusals(src), Vec::<String>::new());
}

#[test]
fn a_call_with_effects_is_refused() {
    one(
        "fn noisy(x: Int) -> Int { println(x); return x; }
@vectorize
fn f(xs: [Int; 8]) -> Int {
    let mut s = 0;
    for x in xs {
        s = s + noisy(x);
    }
    return s;
}
fn main() { println(f([1; 8])); }
",
        "6:17",
        "a call to `noisy`, which has effects (",
    );
}

#[test]
fn a_call_to_a_fn_with_a_loop_is_refused() {
    one(
        "fn upto(n: Int) -> Int { let mut s = 0; for i in 0..n { s = s + i; } return s; }
@vectorize
fn f(xs: [Int; 8]) -> Int {
    let mut s = 0;
    for x in xs {
        s = s + upto(x);
    }
    return s;
}
fn main() { println(f([1; 8])); }
",
        "6:17",
        "which has a loop of its own",
    );
}

#[test]
fn an_allocation_is_refused() {
    one(
        "type P { x: Int; }
@vectorize
fn f(xs: [Int; 8]) -> Int {
    let mut s = 0;
    for x in xs {
        let p = P { x: x };
        s = s + p.x;
    }
    return s;
}
fn main() { println(f([1; 8])); }
",
        "6:17",
        "an allocation (struct `P`)",
    );
}

#[test]
fn a_fn_with_no_loop_is_refused() {
    let r = refusals("@vectorize\nfn f(x: Int) -> Int { return x + 1; }\nfn main() { println(f(1)); }\n");
    assert_eq!(r.len(), 1, "{r:#?}");
    assert!(r[0].starts_with("1:1 ") && r[0].contains("it has no loop"), "{r:#?}");
}

/// Only an innermost loop is held: an outer loop the vectorizer never
/// takes may do what it likes between its inner loops.
#[test]
fn an_outer_loop_is_not_held() {
    let src = "@vectorize
fn f(rows: [[Int; 8]; 4]) -> Int {
    let mut s = 0;
    for row in rows {
        println(s);
        for x in row {
            s = s + x;
        }
    }
    return s;
}
fn main() { println(f([[1; 8]; 4])); }
";
    assert_eq!(refusals(src), Vec::<String>::new());
}
