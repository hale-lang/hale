//! GH #837 — a factory result is owned by the binding that names it,
//! not by the first call lowering reaches.
//!
//! `suppress_fresh_temp` is the one-shot flag a `let`, a `return`, an
//! `=` into a locus slot or a locus-typed field init sets to say "the
//! value this expression produces already has an owner, so do not
//! also register it as the unbound temporary GH #402 reclaims at
//! frame exit". Nothing in the flag said WHICH node it was about, and
//! the taker was whichever proven-fresh factory call lowering reached
//! first — which in
//!
//! ```text
//! return combine(a, make());
//! ```
//!
//! is the ARGUMENT. `make()`'s result was recorded as the caller's,
//! though the caller never sees it, and nothing reclaimed it;
//! `combine`'s result — the value the `return` actually named — was
//! left to the rules for an ordinary call. PR #835 fixed the one
//! spelling that had already bitten (`Expr::Or`, GH #793) by taking
//! the flag up front on the outermost node. This is that fix
//! generalised: the site asks whether the node it NAMES takes the
//! decision, and when it does not, every factory call inside it gets
//! the frame temporary an unowned result is supposed to get.
//!
//! `dissolve()` printing its tag is what makes ownership observable:
//! a missing owner is silence, and two owners are the tag twice.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

/// Every program here answers in milliseconds. The deadline is not
/// about slow machines: a teardown that fires on a value another
/// owner still holds can leave a program spinning instead of exiting,
/// and a bare `Command::output()` would hang a CI job rather than
/// fail it.
const DEADLINE: Duration = Duration::from_secs(60);

fn build_and_run(tag: &str, src: &str) -> (String, String) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("fresh_temp_attr_{}", tag));
    build_executable(&program, &bin).expect("build");
    let mut child = Command::new(&bin)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("run");
    let start = Instant::now();
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(s) => break Some(s),
            None if start.elapsed() > DEADLINE => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    };
    let mut out = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        use std::io::Read;
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        out = String::from_utf8_lossy(&buf).to_string();
    }
    let _ = std::fs::remove_file(&bin);
    let verdict = match status {
        Some(s) if s.success() => String::new(),
        Some(s) => format!("exited {:?}", s),
        None => format!("did not finish within {:?}", DEADLINE),
    };
    (out, verdict)
}

/// A locus whose teardown is observable, a factory for it, and the
/// two kinds of consumer the attribution question turns on: one that
/// does NOT hand the locus back (`tally`) and one that hands back the
/// first of the two it was given (`first`). Neither qualifies as a
/// fresh factory — `tally` does not return a locus at all and
/// `first` returns a parameter — so neither is a node the ownership
/// decision can land on.
const LIB: &str = r#"
    locus Thing {
        params { n: Int = 0; }
        dissolve() { println("bye ", self.n); }
    }

    fn make(n: Int) -> Thing {
        return Thing { n: n };
    }

    fn make_f(n: Int) -> Thing fallible(String) {
        if n < 0 { fail "negative"; }
        return Thing { n: n };
    }

    fn tally(a: Int, t: Thing) -> Int {
        return a + t.n;
    }

    fn first(a: Thing, b: Thing) -> Thing {
        return a;
    }
"#;

/// The headline, in return position. `work`'s frame is the only place
/// `make(5)`'s result can be reclaimed — `tally` hands back an Int,
/// so the caller never sees the locus — and before this fix the
/// `return` gave its ownership decision to that argument and the
/// value was never torn down at all.
#[test]
fn a_return_of_a_plain_call_reclaims_its_factory_argument() {
    let src = format!(
        "{LIB}
        fn work() -> Int {{
            return tally(1, make(5));
        }}
        fn main() {{
            println(\"w=\", work());
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("return_call_arg", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    assert_eq!(
        out, "bye 5\nw=6\nend\n",
        "the factory result passed to a call that does not hand it \
         back is an unbound temporary of the CALLEE's frame, so its \
         teardown runs on the way out of `work` — before the value \
         `work` actually returned is printed"
    );
}

/// The headline, in binding position, with both arguments fresh so
/// that a flag taken by the wrong node is visible as an asymmetry:
/// before the fix the FIRST result was silent (it had taken the
/// binding's decision and no owner came with it) while the second
/// was reclaimed the ordinary way.
#[test]
fn a_binding_of_a_plain_call_dissolves_every_factory_argument_once() {
    let src = format!(
        "{LIB}
        fn main() {{
            let x = first(make(1), make(2));
            println(\"x=\", x.n);
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("let_call_args", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    // Reverse-order flush: `make(2)`'s temporary was registered last.
    assert_eq!(
        out, "x=1\nend\nbye 2\nbye 1\n",
        "`first` is not a factory, so the binding owns nothing and \
         BOTH arguments are temporaries of this frame — each torn \
         down exactly once, after the last use of the binding that \
         aliases one of them"
    );
}

/// The `or` spelling of the same mis-attribution. PR #835 made an
/// `or` whose inner call IS a factory take the decision on the
/// outermost node; an `or` whose inner call is NOT one still passed
/// it down into the arguments, where `make(3)` took it. The fallible
/// wrapper does not hand the locus back either, so this frame is the
/// only owner available.
#[test]
fn an_or_wrapped_plain_call_reclaims_its_factory_argument() {
    let src = format!(
        "{LIB}
        fn tally_f(a: Int, t: Thing) -> Int fallible(String) {{
            if a < 0 {{ fail \"negative\"; }}
            return a + t.n;
        }}
        fn main() {{
            let s = tally_f(1, make(3)) or raise;
            println(\"s=\", s);
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("or_call_arg", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    assert_eq!(
        out, "s=4\nend\nbye 3\n",
        "an `or` around a call that is not a factory decides nothing \
         about the factory in its arguments, which stays this frame's \
         temporary"
    );
}

/// The same decision at a locus-typed FIELD (F.17 / GH #836). The
/// field owns a factory result written straight into it, and nothing
/// else: `pick(make(5), make(6))` hands back a locus `pick` was
/// given, so both results are temporaries of the frame that built
/// them and the field's ownership bit stays clear.
#[test]
fn a_field_init_of_a_plain_call_dissolves_every_factory_argument_once() {
    let src = format!(
        "{LIB}
        locus Router {{
            params {{ quick: Thing = Thing {{ n: 0 }}; tag: Int = 0; }}
            fn peek() -> Int {{ return self.quick.n; }}
            dissolve() {{ println(\"router \", self.tag); }}
        }}
        fn main() {{
            let r = Router {{ quick: first(make(5), make(6)), tag: 1 }};
            println(\"peek=\", r.peek());
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("field_call_args", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    assert_eq!(
        out, "peek=5\nend\nrouter 1\nbye 6\nbye 5\n",
        "the field reads its own value, the owner's cascade does not \
         claim a locus it was merely handed, and each factory result \
         is torn down exactly once"
    );
}

/// The shape the flag exists for, pinned so the narrowing above does
/// not swallow it: a `return` that names the factory call itself
/// hands the result to the CALLER, and the callee's frame contributes
/// no teardown. (`handoff` qualifies as a factory in its own right,
/// so the caller's binding is the single owner.)
#[test]
fn a_returned_factory_result_still_belongs_to_the_caller() {
    let src = format!(
        "{LIB}
        fn handoff(n: Int) -> Thing {{
            return make(n);
        }}
        fn main() {{
            let h = handoff(7);
            println(\"h=\", h.n);
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("return_factory", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    assert_eq!(
        out, "h=7\nend\nbye 7\n",
        "the frame that built the returned value must not reclaim it, \
         and the binding that receives it must — one teardown, at the \
         caller's scope exit"
    );
}

/// A factory whose ARGUMENT is another factory: the outer call takes
/// the decision (it is the node the `let` names) and the inner one
/// takes the frame temporary. Both orders were already right — the
/// GH #402 hook takes the flag on a factory node before descending —
/// and this pins that the site-side narrowing left it that way.
#[test]
fn a_factory_argument_of_a_returned_factory_is_reclaimed() {
    let src = format!(
        "{LIB}
        fn wrap(t: Thing) -> Thing {{
            return Thing {{ n: t.n + 10 }};
        }}
        fn main() {{
            let w = wrap(make(1));
            println(\"w=\", w.n);
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("nested_factory", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    assert_eq!(
        out, "w=11\nend\nbye 11\nbye 1\n",
        "the binding owns the outer factory's result and the frame \
         owns the inner one — two teardowns, neither twice"
    );
}

/// The same defect measured in bytes, from the frame where the waste
/// is LeakSanitizer-visible. A method body's own allocations go to a
/// per-call scratch sub-region, but a factory's locus carries a
/// free-standing `lotus_arena_create_labeled` and its `@form(vec)`
/// buffer is freed only on the dissolve path — so a result nobody
/// owned survived the whole run, once per call. In `main` the same
/// program reads clean (main's arena is destroyed at exit), which is
/// PR #835's note and the reason this test is a method.
///
/// The ASan build goes through `harness::build_asan`
/// (`BuildOptions::asan`, GH #843) — nothing here touches the
/// process environment.
#[test]
fn a_factory_argument_in_a_method_frame_is_leak_clean_under_asan() {
    let src = r#"
        @form(vec)
        locus Buf {
            params { n: Int = 0; }
            capacity { heap data of Float; }
        }

        fn zeros(n: Int) -> Buf {
            let b = Buf { n: n };
            let mut i = 0;
            while i < n { b.push(1.5); i = i + 1; }
            return b;
        }

        fn total(base: Float, b: Buf) -> Float {
            let v = b.get(0) or 0.0;
            return base + v;
        }

        locus Engine {
            params { runs: Int = 0; }
            fn step(n: Int) -> Float {
                self.runs = self.runs + 1;
                return total(1.0, zeros(n));
            }
        }

        fn main() {
            let e = Engine { };
            let mut t = 0.0;
            let mut r = 0;
            while r < 4 { t = t + e.step(16); r = r + 1; }
            println("t=", t);
            println("runs=", e.runs);
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("fresh_temp_attr_asan");
    // `BuildOptions::asan` through the harness — no test mutates the
    // process environment (GH #843), and the helper checks the
    // artifact really carries the ASan runtime.
    harness::build_asan(&program, &bin);
    let out = Command::new(&bin)
        .env("ASAN_OPTIONS", "detect_leaks=1")
        .output()
        .expect("run asan binary");
    let _ = std::fs::remove_file(&bin);
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "non-zero exit under ASan: {:?}\n{}",
        out.status,
        report
    );
    // The values still have to be right: a reclaim that fires while
    // the value is still in use reads back as zeros rather than
    // crashing, which is the failure a sanitizer alone calls success.
    assert!(
        report.contains("t=10") && report.contains("runs=4"),
        "the reclaim disturbed the values it was supposed to \
         outlive:\n{}",
        report
    );
    for bad in [
        "Direct leak",
        "Indirect leak",
        "heap-use-after-free",
        "use-after-free",
        "double-free",
        "attempting double-free",
        "attempting free on address which was not malloc",
        "heap-buffer-overflow",
        "SEGV",
    ] {
        assert!(
            !report.contains(bad),
            "ASan reported `{}`:\n{}",
            bad,
            report
        );
    }
}

// ---------------------------------------------------------------
// GH #883 — the same decision, PER ARM.
//
// `if` / `match` / a block in value position produce no value of
// their own: they hand back an ARM's, and exactly one arm runs.
// PR #882 let the one-shot flag reach them unchanged, so the first
// arm lowered took the site's decision and every other arm's result
// became a GH #402 frame temporary dissolved right here — while the
// owner the site named (a caller, a binding) still held it. Each
// test below runs BOTH paths of the same shape, because a defect
// that lands on "whichever arm lowered first" is invisible from one.
//
// One thing these tests deliberately do not pin: what becomes of a
// returned arm value AFTER it reaches the caller. A fn whose
// `return` names an `if` / `match` / block is not a proven-fresh
// factory — `compute_fresh_locus_factories` classifies the carrier
// node itself, never its arms — so the caller's binding does not
// own the result and nothing reclaims it. That is a leak, it is a
// separate seam, and it was already the fate of the one arm that
// worked before this fix. What #883 is about, and what these
// assertions cover, is that the frame which produced the value no
// longer tears it down while somebody else is still reading it.

/// The headline. `pick` hands back one of two fresh results, and
/// before this fix the `else` arm's was dissolved inside `pick` —
/// `main` then read a field of a torn-down locus.
#[test]
fn an_if_expression_in_return_position_hands_every_arm_to_the_caller() {
    let src = format!(
        "{LIB}
        fn pick(c: Bool) -> Thing {{
            return if c {{ make(1) }} else {{ make(2) }};
        }}
        fn main() {{
            let a = pick(true);
            println(\"a=\", a.n);
            let b = pick(false);
            println(\"b=\", b.n);
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("return_if_arms", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    assert!(
        out.starts_with("a=1\nb=2\nend\n"),
        "no teardown may run inside `pick` on EITHER path: on that \
         path the arm's value is the one the `return` names, so it \
         is the caller's, and a `bye` before the caller's read is a \
         locus torn down under it\n{}",
        out
    );
}

/// Three arms, so "the first one lowered" and "the last one
/// lowered" are both wrong answers and the middle arm has to be
/// reached by the rule rather than by luck.
#[test]
fn a_match_expression_in_return_position_hands_every_arm_to_the_caller() {
    let src = format!(
        "{LIB}
        fn pick(k: Int) -> Thing {{
            return match k {{
                0 -> make(10),
                1 -> make(11),
                _ -> make(12),
            }};
        }}
        fn main() {{
            let a = pick(0);
            println(\"a=\", a.n);
            let b = pick(1);
            println(\"b=\", b.n);
            let c = pick(7);
            println(\"c=\", c.n);
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("return_match_arms", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    assert!(
        out.starts_with("a=10\nb=11\nc=12\nend\n"),
        "every arm of a match in return position hands its value to \
         the caller; before the fix the first arm took the decision \
         and the other two were dissolved inside `pick`\n{}",
        out
    );
}

/// A block in value position, where both halves of the rule show at
/// once: the factory in a STATEMENT is nobody's but this frame's
/// (`bye 100`, on the way out of `pick_b`), and the factory in the
/// TAIL is the value the `return` named, so `pick_b` must not touch
/// it. Before the fix the statement's call took the decision — the
/// flag was still set when the statements lowered — and the tail's
/// result got the frame temporary instead.
#[test]
fn a_block_in_return_position_names_its_tail_and_not_its_statements() {
    let src = format!(
        "{LIB}
        fn pick_b(k: Int) -> Thing {{
            return {{ println(\"t=\", tally(1, make(100))); make(k) }};
        }}
        fn main() {{
            let v = pick_b(7);
            println(\"v=\", v.n);
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("return_block_tail", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    assert!(
        out.starts_with("t=101\nbye 100\nv=7\nend\n"),
        "the statement's factory result is this frame's temporary \
         and is reclaimed on the way out; the tail's is the value \
         the `return` names and must reach the caller intact\n{}",
        out
    );
}

/// The binding half, where the teardown that proves the fix is
/// observable. A `let` registers its scope-exit dissolve only for a
/// direct factory call, so an `if` RHS leaves each arm's result to
/// the GH #402 frame temporary — one per arm, exactly once. Before
/// the fix the then-arm took the binding's suppression and was
/// reclaimed by nobody at all, which is why only the second
/// evaluation's teardown printed.
#[test]
fn a_binding_of_an_if_expression_reclaims_every_arm_exactly_once() {
    let src = format!(
        "{LIB}
        fn main() {{
            let c = true;
            let x = if c {{ make(1) }} else {{ make(2) }};
            println(\"x=\", x.n);
            let d = false;
            let y = if d {{ make(3) }} else {{ make(4) }};
            println(\"y=\", y.n);
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("let_if_arms", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    assert_eq!(
        out, "x=1\ny=4\nend\nbye 4\nbye 1\n",
        "each evaluated arm's result is reclaimed once, at this \
         frame's exit and in reverse registration order — the \
         then-arm no longer vanishes without an owner"
    );
}

/// The composite-hint arms of `lower_expr_into`, which descend into
/// an ascribed array's or tuple's elements without passing the
/// outer node through `lower_expr` — so the GH #402 hook, the one
/// place that takes the flag, never sees the node the site named.
/// An aggregate is not any one of its elements: whatever owns it
/// owns the storage, so the decision stops at the composite and
/// every element factory takes the frame temporary. This shape
/// reads the same before and after the fix (no site arms the flag
/// for an array or tuple literal today); it pins the rule so the
/// first site that does cannot hand an aggregate's ownership to its
/// first element.
#[test]
fn an_ascribed_array_or_tuple_reclaims_every_element_exactly_once() {
    let src = format!(
        "{LIB}
        fn main() {{
            let xs: [Thing; 2] = [make(1), make(2)];
            println(\"x0=\", xs[0].n);
            let pr: (Thing, Thing) = (make(3), make(4));
            println(\"p0=\", pr.0.n);
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("composite_elements", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    assert_eq!(
        out, "x0=1\np0=3\nend\nbye 4\nbye 3\nbye 2\nbye 1\n",
        "all four element factories are temporaries of this frame — \
         four teardowns, none twice, none missing"
    );
}

/// The per-arm defect measured from a method frame, where the waste
/// is LeakSanitizer-visible for the reason PR #835 documented: a
/// `@form(vec)` locus carries its own arena and its buffer is freed
/// only on the dissolve path, so an arm result nobody owned
/// survives the whole run. `let b = if …` is the shape — the
/// binding registers no dissolve for a carrier RHS, so before the
/// fix the then-arm's `zeros(n)` took the binding's suppression and
/// leaked once per call while the else-arm was reclaimed normally.
///
/// The build goes through `harness::build_asan` (`BuildOptions::
/// asan`, GH #843), whose runtime cflags carry
/// `-DLOTUS_NO_CHUNK_POOL_DEFAULT=1` (GH #816); the child states
/// `LOTUS_NO_CHUNK_POOL=1` as well, so chunk recycling cannot mask
/// a reclaim that fires on a value still in use.
#[test]
fn an_if_expression_binding_in_a_method_frame_is_leak_clean_under_asan() {
    let src = r#"
        @form(vec)
        locus Buf {
            params { n: Int = 0; }
            capacity { heap data of Float; }
        }

        fn zeros(n: Int) -> Buf {
            let b = Buf { n: n };
            let mut i = 0;
            while i < n { b.push(1.5); i = i + 1; }
            return b;
        }

        fn total(base: Float, b: Buf) -> Float {
            let v = b.get(0) or 0.0;
            return base + v;
        }

        locus Engine {
            params { runs: Int = 0; }
            fn step(n: Int) -> Float {
                self.runs = self.runs + 1;
                let b = if n > 8 { zeros(n) } else { zeros(4) };
                return total(1.0, b);
            }
        }

        fn main() {
            let e = Engine { };
            let mut t = 0.0;
            let mut r = 0;
            while r < 4 { t = t + e.step(16); r = r + 1; }
            println("t=", t);
            println("runs=", e.runs);
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("fresh_temp_arm_asan");
    harness::build_asan(&program, &bin);
    let out = Command::new(&bin)
        .env("ASAN_OPTIONS", "detect_leaks=1")
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run asan binary");
    let _ = std::fs::remove_file(&bin);
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success(),
        "non-zero exit under ASan: {:?}\n{}",
        out.status,
        report
    );
    assert!(
        report.contains("t=10") && report.contains("runs=4"),
        "the reclaim disturbed the values it was supposed to \
         outlive:\n{}",
        report
    );
    for bad in [
        "Direct leak",
        "Indirect leak",
        "heap-use-after-free",
        "use-after-free",
        "double-free",
        "attempting double-free",
        "attempting free on address which was not malloc",
        "heap-buffer-overflow",
        "SEGV",
    ] {
        assert!(
            !report.contains(bad),
            "ASan reported `{}`:\n{}",
            bad,
            report
        );
    }
}
