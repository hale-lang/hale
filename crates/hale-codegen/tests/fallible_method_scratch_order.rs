//! A fallible locus method must dissolve its deferred loci BEFORE it
//! destroys its method scratch — not after.
//!
//! A free-fn factory allocates its locus out of the caller's
//! published arena, and a method publishes its own per-call scratch
//! subregion. So a factory result let-bound inside a method frame
//! has its *struct* living in that scratch. The binding also owns it
//! (GH #383), so the frame's exit dissolves it — which loads the
//! locus's `__arena` field and destroys that arena.
//!
//! Do those in the wrong order and the dissolve reads a struct out
//! of a region that was just reclaimed and, via the subregion
//! freelist, may already be handed back out. The garbage it finds in
//! the `__arena` slot goes straight to `lotus_arena_destroy`.
//!
//! Every other method epilogue flushed before destroying. The
//! FALLIBLE epilogue alone had them reversed, so the bug needed all
//! three of: a locus method, declared `fallible`, that let-binds a
//! factory result. A downstream trainer hit exactly that shape — its
//! `fit(...) -> () fallible(E)` preallocated two row buffers — and
//! segfaulted in `lotus_arena_destroy` at method exit having already
//! computed entirely correct results, which is the worst way for
//! this to present: the numbers look right up to the crash.
//!
//! The non-fallible twin is here too. It always worked, and it is
//! what made the bug look like it was about factories or nesting
//! rather than about `fallible`.

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;

use hale_codegen::build_executable;

fn run(name: &str, src: &str) -> (String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(name);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (String::from_utf8_lossy(&out.stdout).to_string(), out.status)
}

/// Build and return the emitted LLVM IR alongside the run result.
///
/// The ordering this file pins is a use-after-free, and a UAF only
/// *crashes* when the freed bytes have actually been reused — at
/// this program's size the old contents usually survive, so the
/// broken order still exits 0 with correct output. (That is exactly
/// how it stayed hidden: the downstream report needed 5000 epochs of
/// churn to turn it into a segfault.) Asserting on the emitted order
/// is deterministic where asserting on a crash is not.
fn ir_and_run(
    name: &str,
    src: &str,
) -> (String, String, std::process::ExitStatus) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(name);
    // Set-only, never unset: every test in this binary wants it, so
    // parallel execution cannot observe a half-applied value. The
    // dump path derives from `bin`, which is already unique per test.
    unsafe { std::env::set_var("LOTUS_DUMP_IR", "1") };
    build_executable(&program, &bin).expect("build");
    let ir = std::fs::read_to_string(bin.with_extension("ll"))
        .expect("LOTUS_DUMP_IR should have written a .ll");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    let _ = std::fs::remove_file(bin.with_extension("ll"));
    (
        ir,
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

/// The body of `define ... @<name>(`, up to its closing brace.
fn function_body<'a>(ir: &'a str, name: &str) -> &'a str {
    let needle = format!("@{}(", name);
    let start = ir
        .find(&needle)
        .unwrap_or_else(|| panic!("no definition of {} in IR", name));
    let rest = &ir[start..];
    let end = rest.find("\n}").unwrap_or(rest.len());
    &rest[..end]
}

/// `fit` is fallible and let-binds two factory results, then keeps
/// using them across a loop — the downstream trainer's shape.
const FALLIBLE: &str = r#"
@form(vec)
locus Row {
    params { n: Int = 0; }
    capacity { heap data of Float; }
}

fn make_row(v: Float) -> Row {
    let r = Row { n: 1 };
    r.push(v);
    return r;
}

type StepError { detail: String = ""; }

locus Model {
    params { total: Float = 0.0; }
    fn step(a: Row, b: Row) -> Float fallible(StepError) {
        if (a.get(0) or 0.0) < 0.0 {
            fail StepError { detail: "negative" };
        }
        self.total = self.total + (a.get(0) or 0.0) + (b.get(0) or 0.0);
        return (a.get(0) or 0.0) + (b.get(0) or 0.0);
    }
}

locus Trainer {
    params { rounds: Int = 500; }
    fn fit(m: Model) -> () fallible(StepError) {
        let x = make_row(1.0);
        let y = make_row(2.0);
        let mut i = 0;
        while i < self.rounds {
            let s = m.step(x, y) or raise;
            i = i + 1;
        }
    }
}

fn report(e: StepError) {
    println("fit failed: ", e.detail);
}

fn main() {
    let m = Model { };
    let t = Trainer { };
    t.fit(m) or report(err);
    println("total=", to_string(m.total));
    let z = make_row(9.0);
    println("post=", to_string(z.get(0) or 0.0));
}
"#;

#[test]
fn a_fallible_method_dissolves_before_destroying_its_scratch() {
    let (ir, out, status) =
        ir_and_run("fallible_scratch_order", FALLIBLE);
    let body = function_body(&ir, "Trainer.fit");

    let scratch_destroy = body
        .find("lotus_arena_destroy(ptr %method.scratch.load)")
        .expect("fit should destroy its method scratch");
    let first_dissolve_read = body
        .find("dissolve.arena.gep")
        .expect("fit should dissolve its two factory-bound loci");

    assert!(
        first_dissolve_read < scratch_destroy,
        "the deferred dissolve reads a locus struct at offset {} but \
         the method scratch holding that struct is destroyed at \
         offset {} — the dissolve is loading `__arena` out of freed \
         memory.\n\nA factory allocates in the caller's published \
         arena, and a method publishes its scratch, so these structs \
         LIVE in the region being destroyed.",
        first_dissolve_read,
        scratch_destroy
    );

    assert!(
        status.success(),
        "fallible method exited {:?} — a crash here is the scratch \
         being destroyed before the deferred dissolves read the \
         locus structs living in it.\nstdout:\n{}",
        status.code(),
        out
    );
    // Values, not just survival: an ordering fix that quietly
    // dissolved the wrong thing would still exit 0 while handing
    // back zeros, and that failure mode has burned this area before.
    assert!(
        out.contains("total=1500"),
        "expected total=1500 (500 rounds x 3.0); got:\n{}",
        out
    );
    assert!(
        out.contains("post=9"),
        "allocation after the fallible method must still work; got:\n{}",
        out
    );
}

/// The same program with `fallible` removed. This path always
/// flushed before destroying, and pinning it keeps the two epilogues
/// from drifting apart again.
const NON_FALLIBLE: &str = r#"
@form(vec)
locus Row {
    params { n: Int = 0; }
    capacity { heap data of Float; }
}

fn make_row(v: Float) -> Row {
    let r = Row { n: 1 };
    r.push(v);
    return r;
}

locus Model {
    params { total: Float = 0.0; }
    fn step(a: Row, b: Row) {
        self.total = self.total + (a.get(0) or 0.0) + (b.get(0) or 0.0);
    }
}

locus Trainer {
    params { rounds: Int = 500; }
    fn fit(m: Model) {
        let x = make_row(1.0);
        let y = make_row(2.0);
        let mut i = 0;
        while i < self.rounds {
            m.step(x, y);
            i = i + 1;
        }
    }
}

fn main() {
    let m = Model { };
    let t = Trainer { };
    t.fit(m);
    println("total=", to_string(m.total));
    let z = make_row(9.0);
    println("post=", to_string(z.get(0) or 0.0));
}
"#;

#[test]
fn the_non_fallible_twin_behaves_identically() {
    let (out, status) = run("nonfallible_scratch_order", NON_FALLIBLE);
    assert!(
        status.success(),
        "non-fallible method exited {:?}\nstdout:\n{}",
        status.code(),
        out
    );
    assert!(
        out.contains("total=1500"),
        "expected total=1500; got:\n{}",
        out
    );
    assert!(out.contains("post=9"), "got:\n{}", out);
}

/// Three levels of locus nesting where the middle one calls a
/// free-fn factory. This failed to BUILD at all — LLVM rejected the
/// module ("!dbg attachment points at wrong subprogram") because the
/// prologue of the outer method inherited the debug location left
/// live by the previously emitted method. Debug info is always on,
/// so there was no way to compile it.
const THREE_LEVELS: &str = r#"
locus Sample {
    params { a: Float = 0.0; }
}

fn make(a: Float) -> Sample {
    return Sample { a: a };
}

locus Model {
    params { total: Float = 0.0; }
    fn train_step(s: Sample) { self.total = self.total + s.a; }
}

locus Trainer {
    params { rounds: Int = 100; }
    fn fit(model: Model) {
        let mut i = 0;
        while i < self.rounds {
            let x = make(1.0);
            model.train_step(x);
            i = i + 1;
        }
    }
}

locus Demo {
    params {}
    fn go() {
        let m = Model { };
        let t = Trainer { };
        t.fit(m);
        println("total=", to_string(m.total));
    }
}

fn main() {
    let d = Demo { };
    d.go();
}
"#;

#[test]
fn three_levels_of_locus_nesting_compile_and_run() {
    let (out, status) = run("three_level_nesting", THREE_LEVELS);
    assert!(status.success(), "exited {:?}\n{}", status.code(), out);
    assert!(out.contains("total=100"), "got:\n{}", out);
}
