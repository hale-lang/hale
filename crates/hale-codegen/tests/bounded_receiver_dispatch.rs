//! GH #892 — a free `fn` named after a `bounded[T; N]` intrinsic runs
//! its own body when the argument is a bounded receiver.
//!
//! `count` / `clear` / `truncate` / `push` / `at` / `set` are
//! intrinsics only where the first argument IS a `bounded[T; N]`.
//! That is why the parser's `BUILTIN_CALL_FORMS` (GH #880, PR #891)
//! deliberately does not claim them — `dna/tests/books_slice_test.hl`
//! declares a free `fn count(app, kind, entity, needle)` and calls it
//! — and outside a bounded argument the declaration already won.
//!
//! On a bounded argument it did not, and nothing said so. The issue's
//! reproducer:
//!
//! ```hale
//! type Window { id: String; samples: bounded[Int; 8]; }
//! fn count(xs: bounded[Int; 8]) -> Int { return 8801; }
//! fn main() {
//!     let mut w = Window { id: "w1" };
//!     push(w.samples, 1) or { };
//!     println("v=", count(w.samples));
//! }
//! ```
//!
//! printed `v=1` — the live count. Both layers dispatched on the
//! ARGUMENT's type ahead of the program's own fns, so `hale check`
//! typed the call as the intrinsic too; the program only checked at
//! all because both answers happen to be `Int`. Give the declaration
//! a `String` return and `hale check` refused a program whose author
//! had written nothing wrong.
//!
//! The rule now, in both layers: **a declaration whose first
//! parameter is the receiver's own `bounded[T; N]` answers the
//! call**, at any arity it accepts. Dispatch is type-directed, so the
//! shadow is decidable at the call site — which is what makes this
//! resolvable in the program's favour where GH #880's flat-namespace
//! names (`abs`, `min`, the printers) could only be refused.
//!
//! Element type and capacity are part of the match on purpose: the
//! Hale-source standard library is merged into the same global fn
//! namespace and calls these intrinsics on buffers of its own, so a
//! name-only shadow retargeted the LIBRARY's calls at the user's
//! declaration — the `print` capture, one layer down. A first probe
//! of this fix did exactly that and died with ``fn `count` arg 0 type
//! mismatch: expected Bounded(Int, 8), got Bounded(Float, 32)`` out
//! of `metrics.hl`.
//!
//! Method position is untouched throughout: `f.get(i)` and the
//! element-chain terminals are reached through a receiver, which no
//! free fn claims.

use std::process::Command;

use hale_codegen::build_executable;
use hale_syntax::parse_source;

#[path = "support/harness.rs"]
mod harness;

/// Typecheck, build and run `src`; return its stdout.
///
/// The check is part of the assertion, not a convenience: the whole
/// point of the rule is that `hale check` and `hale build` resolve
/// the call to the SAME fn, and `build_executable` does not run the
/// checker.
fn check_build_run(name: &str, src: &str) -> String {
    let program = parse_source(src).expect("parse");
    let errs: Vec<String> = hale_types::check_program(&program)
        .iter()
        .filter(|d| d.is_error())
        .map(|d| d.message.clone())
        .collect();
    assert!(errs.is_empty(), "`hale check` refuses it: {:?}", errs);
    let bin = harness::unique_bin(name);
    build_executable(&program, &bin).expect("build");
    let out = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    assert!(out.status.success(), "non-zero: {:?}", out.status);
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The issue's program, verbatim apart from the `or` arm's spelling.
#[test]
fn the_issues_reproducer_runs_its_own_count() {
    let out = check_build_run(
        "bounded_shadow_issue",
        r#"
        type Window { id: String; samples: bounded[Int; 8]; }
        fn count(xs: bounded[Int; 8]) -> Int { return 8801; }
        fn main() {
            let mut w = Window { id: "w1" };
            push(w.samples, 1) or raise;
            println("v=", count(w.samples));
        }
    "#,
    );
    assert!(
        out.contains("v=8801"),
        "the declaration did not answer the call: {:?}",
        out
    );
}

/// All six names, each declared at the arity its intrinsic takes, each
/// called on a bounded receiver — in one program, so a fix that only
/// reached one of the three dispatch sites (statement position,
/// expression position, the `or`/fallible path) fails here.
#[test]
fn each_of_the_six_names_runs_its_own_body() {
    let out = check_build_run(
        "bounded_shadow_six",
        r#"
        type W { id: String; s: bounded[Int; 8]; }

        fn count(xs: bounded[Int; 8]) -> Int { return 8801; }
        fn clear(xs: bounded[Int; 8]) -> Int { return 8802; }
        fn truncate(xs: bounded[Int; 8], n: Int) -> Int { return 8803; }
        fn push(xs: bounded[Int; 8], x: Int) -> Int { return 8804; }
        fn at(xs: bounded[Int; 8], i: Int) -> Int { return 8805; }
        fn set(xs: bounded[Int; 8], i: Int, x: Int) -> Int { return 8806; }

        fn main() {
            let mut w = W { id: "w1" };
            println("count=", count(w.s));
            println("clear=", clear(w.s));
            println("truncate=", truncate(w.s, 1));
            println("push=", push(w.s, 1));
            println("at=", at(w.s, 0));
            println("set=", set(w.s, 0, 9));
            clear(w.s);
            truncate(w.s, 2);
            println("still=", count(w.s));
        }
    "#,
    );
    for want in [
        "count=8801",
        "clear=8802",
        "truncate=8803",
        "push=8804",
        "at=8805",
        "set=8806",
        // The two statement-position calls ran the bodies, so nothing
        // mutated the buffer.
        "still=8801",
    ] {
        assert!(out.contains(want), "missing {:?} in {:?}", want, out);
    }
}

/// The other half of the receiver surface: a locus `params` field
/// reached through `self`, inside a method.
///
/// The intrinsic's receiver is any bounded-typed lvalue, and the
/// shadow has to reach the same set — codegen resolves `self.recent`
/// through `static_expr_codegen_ty`, the checker through its own
/// field lookup, and a rule held in only one of them would put the
/// two layers back in disagreement.
#[test]
fn a_self_params_receiver_reaches_the_declaration_too() {
    let out = check_build_run(
        "bounded_shadow_self",
        r#"
        fn count(xs: bounded[Float; 4]) -> Int { return 8801; }

        locus Tracker {
            params { name: String = "t"; recent: bounded[Float; 4]; }
            fn note(v: Float) -> Int {
                push(self.recent, v) or raise;
                return count(self.recent);
            }
        }

        fn main() {
            let t = Tracker { };
            println("n=", t.note(1.0));
        }
    "#,
    );
    assert!(out.contains("n=8801"), "{:?}", out);
}

/// The control: with nothing declared, the intrinsics still answer,
/// with their real semantics. The rule must not widen into "a bounded
/// receiver no longer dispatches".
#[test]
fn the_intrinsics_answer_when_nothing_declares_the_name() {
    let out = check_build_run(
        "bounded_shadow_control",
        r#"
        type W { id: String; s: bounded[Int; 8]; }
        fn main() {
            let mut w = W { id: "w1" };
            push(w.s, 5) or raise;
            push(w.s, 6) or raise;
            println("count=", count(w.s));
            println("at1=", at(w.s, 1) or 0);
            set(w.s, 0, 9) or raise;
            println("at0=", at(w.s, 0) or 0);
            println("trunc=", truncate(w.s, 1));
            clear(w.s);
            println("cleared=", count(w.s));
        }
    "#,
    );
    for want in
        ["count=2", "at1=6", "at0=9", "trunc=1", "cleared=0"]
    {
        assert!(out.contains(want), "missing {:?} in {:?}", want, out);
    }
}

/// The shadow follows the receiver TYPE, not just the name — the two
/// spellings live side by side in one program.
///
/// This is what keeps the standard library's own intrinsic calls out
/// of a user declaration's reach: `http.hl` pushes onto
/// `bounded[String; 8]`, `metrics.hl` onto `bounded[Float; 32]` and
/// `bounded[Int; 33]`, and none of those is the author's type.
#[test]
fn a_declaration_over_a_different_bounded_does_not_shadow() {
    let out = check_build_run(
        "bounded_shadow_narrow",
        r#"
        type W { id: String; wide: bounded[Int; 8]; narrow: bounded[Int; 4]; }
        fn count(xs: bounded[Int; 4]) -> Int { return 8801; }
        fn main() {
            let mut w = W { id: "w1" };
            push(w.wide, 1) or raise;
            push(w.wide, 2) or raise;
            println("wide=", count(w.wide));
            println("narrow=", count(w.narrow));
        }
    "#,
    );
    assert!(
        out.contains("wide=2"),
        "the intrinsic must still answer a receiver the declaration \
         does not take: {:?}",
        out
    );
    assert!(
        out.contains("narrow=8801"),
        "the declaration must answer its own receiver type: {:?}",
        out
    );
}

/// The DNA fixture's shape: a free `fn count` over four `String`s,
/// beside a bounded receiver in the same program.
///
/// `dna/tests/books_slice_test.hl:55` is the real one. Its
/// declaration takes no bounded parameter at all, so it neither
/// shadows the intrinsic nor is shadowed BY it — both spellings
/// resolve, which is the property the rule must not break.
#[test]
fn a_four_string_count_and_the_intrinsic_coexist() {
    let out = check_build_run(
        "bounded_shadow_dna_shape",
        r#"
        type W { id: String; s: bounded[Int; 8]; }

        // Rows of `kind` about `entity` whose body contains `needle`.
        fn count(app: String, kind: String, entity: String, needle: String) -> Int {
            return len(app) + len(kind) + len(entity) + len(needle);
        }

        fn main() {
            let mut w = W { id: "w1" };
            push(w.s, 1) or raise;
            push(w.s, 2) or raise;
            println("rows=", count("ab", "cd", "e", ""));
            println("live=", count(w.s));
        }
    "#,
    );
    assert!(
        out.contains("rows=5"),
        "the four-String declaration must still be called: {:?}",
        out
    );
    assert!(
        out.contains("live=2"),
        "the intrinsic must still answer the bounded receiver: {:?}",
        out
    );
}

/// The check/build agreement, stated where it can fail.
///
/// A declaration that returns something the intrinsic does not is the
/// only shape where the two layers could disagree in silence. Before
/// the fix the checker typed this call `Int` and refused the program;
/// the built binary, had it got that far, would have printed the live
/// count. Now both resolve to the declaration.
#[test]
fn check_and_build_agree_when_the_return_types_differ() {
    let out = check_build_run(
        "bounded_shadow_return_ty",
        r#"
        type W { id: String; s: bounded[Int; 8]; }
        fn count(xs: bounded[Int; 8]) -> String { return "mine"; }
        fn main() {
            let mut w = W { id: "w1" };
            push(w.s, 1) or raise;
            let v: String = count(w.s);
            println("v=", v);
        }
    "#,
    );
    assert!(out.contains("v=mine"), "{:?}", out);
}

/// Method position is not a free-fn namespace, so a declared `fn at`
/// does not change what `f.get(i)` means — nor what an element chain
/// terminal means.
///
/// `get` is the chain source protocol (spec/types.md § `bounded[T;
/// N]`): the checker types it off the RECEIVER, and codegen routes it
/// into the intrinsic directly. Had the `get` path gone through the
/// shadow check, `hale check` and `hale build` would have disagreed
/// about a program that declares `fn at`.
#[test]
fn get_and_chain_terminals_keep_the_intrinsic() {
    let out = check_build_run(
        "bounded_shadow_get",
        r#"
        type W { id: String; s: bounded[Int; 8]; }
        fn at(xs: bounded[Int; 8], i: Int) -> Int { return 8805; }
        fn count(xs: bounded[Int; 8]) -> Int { return 8801; }
        fn main() {
            let mut w = W { id: "w1" };
            push(w.s, 11) or raise;
            push(w.s, 22) or raise;
            println("at=", at(w.s, 1));
            println("get=", w.s.get(1) or 0);
            println("chain=", w.s.filter(it > 10).count());
        }
    "#,
    );
    for want in ["at=8805", "get=22", "chain=2"] {
        assert!(out.contains(want), "missing {:?} in {:?}", want, out);
    }
}
