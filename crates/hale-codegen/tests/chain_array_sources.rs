//! Element chains over the type-level collections: `[T; N]` and
//! `bounded[T; N]`.
//!
//! Chains desugar — before typecheck, so with no type to dispatch on
//! — into a loop that fetches each element through the source's
//! `get`. A `@form(vec)` is a locus and has that method; a fixed
//! array and a `bounded` are types, whose operations are free
//! intrinsics (`at(f, i)`), so the desugared loop hit "no field
//! `get`" and chains could not anchor on them at all. A downstream
//! fleet counted ~11 would-be sites in one component and ~44
//! hand-rolled index walks across the whole thing.
//!
//! `get(Int) -> T fallible(IndexError)` now answers on both, which
//! is the accessor the desugar was already built around.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn run_src(name: &str, source: &str) -> String {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("hale_test_chainsrc_{}", name));
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(
        out.status.success(),
        "program must run: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// The acceptance test the friction report asked for: a fixed array
/// `.filter(…).count()` compiles and counts.
#[test]
fn a_fixed_array_is_a_chain_source() {
    let out = run_src(
        "array",
        r#"
type Probe { live: Bool = false; id: Int = 0; }
locus Fleet {
    params {
        ps: [Probe; 4] = [
            Probe { live: true,  id: 1 },
            Probe { live: false, id: 2 },
            Probe { live: true,  id: 3 },
            Probe { live: true,  id: 4 },
        ];
    }
    run {
        println("live=", self.ps.filter(it.live).count());
        println("sum=", self.ps.map(it.id).sum(0));
        println("any=", self.ps.any(it.id == 3));
        println("all=", self.ps.all(it.id > 0));
    }
}
fn main() { let f = Fleet { }; }
"#,
    );
    assert!(out.contains("live=3"), "{:?}", out);
    // Every slot of a fixed array is live, so the walk must cover
    // all N — a chain that stopped early would still look plausible
    // on `count` alone.
    assert!(out.contains("sum=10"), "1+2+3+4: {:?}", out);
    assert!(out.contains("any=true"), "{:?}", out);
    assert!(out.contains("all=true"), "{:?}", out);
}

/// `bounded` walks its LIVE slots, not its capacity. It routes into
/// the same `at` the free intrinsic uses, which is bounded by `len`
/// — a second bounds check written here could have used the
/// capacity and silently read uninitialized slots.
#[test]
fn a_bounded_chain_walks_live_slots_not_capacity() {
    let out = run_src(
        "bounded",
        r#"
locus Fleet {
    params { xs: bounded[Int; 8]; }
    birth() {
        push(self.xs, 5) or discard;
        push(self.xs, 7) or discard;
        push(self.xs, 9) or discard;
    }
    run {
        println("count=", self.xs.filter(it > 5).count());
        println("sum=", self.xs.map(it).sum(0));
        println("any=", self.xs.any(it == 7));
    }
}
fn main() { let f = Fleet { }; }
"#,
    );
    assert!(out.contains("count=2"), "7 and 9: {:?}", out);
    assert!(
        out.contains("sum=21"),
        "5+7+9 and nothing from the 5 unused slots: {:?}",
        out
    );
    assert!(out.contains("any=true"), "{:?}", out);
}

/// An empty `bounded` has zero live slots; the chain must terminate
/// immediately rather than read capacity.
#[test]
fn an_empty_bounded_chains_to_zero() {
    let out = run_src(
        "boundedempty",
        r#"
locus Fleet {
    params { xs: bounded[Int; 4]; }
    run {
        println("count=", self.xs.filter(it > 0).count());
        println("sum=", self.xs.map(it).sum(0));
        println("any=", self.xs.any(it == 1));
    }
}
fn main() { let f = Fleet { }; }
"#,
    );
    assert!(out.contains("count=0"), "{:?}", out);
    assert!(out.contains("sum=0"), "{:?}", out);
    assert!(out.contains("any=false"), "{:?}", out);
}

/// `get` is the accessor the desugar rides, and it is also directly
/// callable — it is fallible, so an out-of-range index takes the
/// `or` arm rather than reading past the end.
#[test]
fn get_is_fallible_on_both_source_forms() {
    let out = run_src(
        "getdirect",
        r#"
locus Fleet {
    params { ps: [Int; 3] = [10, 20, 30]; xs: bounded[Int; 4]; }
    birth() { push(self.xs, 42) or discard; }
    run {
        println("a=", self.ps.get(1) or 0);
        println("b=", self.ps.get(9) or -1);
        println("c=", self.ps.get(0 - 1) or -2);
        println("d=", self.xs.get(0) or 0);
        println("e=", self.xs.get(3) or -1);
    }
}
fn main() { let f = Fleet { }; }
"#,
    );
    assert!(out.contains("a=20"), "{:?}", out);
    assert!(out.contains("b=-1"), "past the end: {:?}", out);
    assert!(out.contains("c=-2"), "negative index: {:?}", out);
    assert!(out.contains("d=42"), "{:?}", out);
    assert!(
        out.contains("e=-1"),
        "past the live count, inside capacity: {:?}",
        out
    );
}
