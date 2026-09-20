//! GH #793 — a factory-returned locus reached through `or` has an
//! owner too.
//!
//! `let c = std::process::spawn("sleep\n30") or raise;` left the child
//! running with PPID 1 after the program exited, its three pipe fds
//! open and ~216 bytes of arena abandoned per spawn — and the same for
//! any factory-returned locus, not only `Child`.
//!
//! The rule that should have covered it was already there. GH #383
//! gave a `let`-bound factory result an owner (the binding) and GH
//! #402 gave an unbound temporary one (the enclosing frame); both
//! match on `Expr::Call`, and `make() or raise` is an `Expr::Or`. So
//! every fallible factory — which is most of the interesting ones,
//! because a factory that can fail is exactly the kind that holds a
//! file descriptor, a socket or a process — fell through both rules
//! and went to the program-lifetime path.
//!
//! The owner is now registered on the `or`'s OK branch, in a slot the
//! err branch never stores to. That placement is the whole design:
//!
//!   * `or raise` / `or fail` diverge, so the binding can only ever
//!     hold the call's result;
//!   * `or <substitute>` produces a DIFFERENT value on the other
//!     branch, and that value already has an owner of its own (an
//!     unowned literal registers itself — GH #814; a fresh factory
//!     call in that position takes a temporary — GH #402). A slot
//!     written only on the ok branch therefore reclaims exactly the
//!     value that was produced, exactly once, either way.
//!
//! The two shapes where the frame must NOT reclaim are unchanged and
//! pinned below: a result the parent owns through a param field (F.17)
//! and a binding the enclosing fn hands back to its caller (the
//! expensive guard from GH #383 — dissolving it gives the caller a
//! dead locus that reads back as zeros rather than crashing).
//!
//! `dissolve()` printing its tag is what makes all of this visible; a
//! missing teardown is silence, and a double teardown is the tag
//! twice.

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
    let bin = harness::unique_bin(&format!("factory_or_{}", tag));
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

/// A locus whose teardown is observable, plus a fallible factory for
/// it and the infallible twin the `or` form is measured against.
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
"#;

/// The headline. Both spellings of the same factory have to be the
/// same program: the `or`-wrapped one used to print nothing at all.
#[test]
fn a_let_bound_or_wrapped_factory_result_dissolves_at_scope_exit() {
    let src = format!(
        "{LIB}
        fn main() {{
            let a = make(1);
            println(\"a=\", a.n);
            let b = make_f(2) or raise;
            println(\"b=\", b.n);
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("let_or_raise", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    // Reverse-order flush: `b` was registered after `a`.
    assert_eq!(
        out, "a=1\nb=2\nend\nbye 2\nbye 1\n",
        "the `or`-wrapped factory result must be reclaimed by the \
         binding's scope, exactly as the bare call is, and only after \
         the last use of the binding"
    );
}

/// `or <substitute>`: whichever branch ran, ONE value is torn down.
/// This is the shape the ok-branch slot exists for — registering the
/// binding itself would dissolve the substitute literal twice, since
/// an unowned literal already owns itself (GH #814).
#[test]
fn an_or_substitute_tears_down_exactly_the_value_that_was_produced() {
    let src = format!(
        "{LIB}
        fn main() {{
            let ok = make_f(2) or Thing {{ n: 99 }};
            println(\"ok=\", ok.n);
            let bad = make_f(0 - 1) or Thing {{ n: 98 }};
            println(\"bad=\", bad.n);
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("or_substitute", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    // `99` is never constructed (the substitute is lazy), `2` comes
    // from the factory and `98` from the substitute literal — one
    // teardown each, neither twice.
    assert_eq!(
        out, "ok=2\nbad=98\nend\nbye 98\nbye 2\n",
        "exactly one of the two branch values must be torn down per \
         binding, and the unused substitute must not be built at all"
    );
}

/// F.17 (GH #583): a factory result written as a locus-typed FIELD of
/// a literal belongs to that literal, in the `or` spelling as much as
/// the bare one. The enclosing frame must not reclaim it — the field
/// still points at it, and its owner tears it down.
#[test]
fn a_factory_result_in_a_param_field_is_owned_by_the_literal() {
    let src = format!(
        "{LIB}
        locus Router {{
            params {{ quick: Thing = Thing {{ n: 0 }}; tag: Int = 0; }}
            fn peek() -> Int {{ return self.quick.n; }}
            dissolve() {{ println(\"router \", self.tag); }}
        }}
        fn main() {{
            let bare = Router {{ quick: make(5), tag: 1 }};
            let wrapped = Router {{ quick: make_f(6) or raise, tag: 2 }};
            println(\"bare=\", bare.peek());
            println(\"wrapped=\", wrapped.peek());
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("field_owned", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    assert_eq!(
        out,
        "bare=5\nwrapped=6\nend\nrouter 2\nrouter 1\n",
        "the two spellings must agree: the field reads its own value \
         after the statement that built it, and the frame contributes \
         no teardown of its own for either"
    );
}

/// The guard that cost the most to find on GH #383, in the `or`
/// spelling: a fn that binds a factory result and RETURNS it hands
/// ownership to its caller. Dissolving it here would give the caller
/// a dead locus — which reads back as zeros rather than crashing, so
/// it needs a value assertion and not a sanitizer.
#[test]
fn a_binding_the_fn_hands_back_is_not_dissolved_by_its_own_frame() {
    let src = format!(
        "{LIB}
        // Fails the freshness walk (its return is a binding whose
        // initializer is an `or`, not a literal), but still hands
        // back a locus it bound from a factory.
        fn relay(n: Int) -> Thing {{
            let c = make_f(n) or Thing {{ n: 0 }};
            return c;
        }}
        fn main() {{
            let h = relay(7);
            println(\"h=\", h.n);
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("returned_binding", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    assert!(
        out.starts_with("h=7\nend\n"),
        "the returned binding must reach the caller alive, not \
         dissolved by the frame that built it: {:?}",
        out
    );
    assert!(
        !out.contains("bye 7"),
        "`relay`'s frame tore down the value it handed back: {:?}",
        out
    );
}

/// GH #815's per-iteration reclaim reaches the `or` form too: the
/// slot is one alloca per SITE, so without it a loop of factory calls
/// would leave the scope-exit flush holding only the LAST result.
#[test]
fn a_loop_of_or_wrapped_factory_calls_dissolves_every_iteration() {
    let src = format!(
        "{LIB}
        fn main() {{
            let mut i = 0;
            while i < 3 {{
                let t = make_f(100 + i) or raise;
                println(\"loop=\", t.n);
                i = i + 1;
            }}
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("loop_or", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    assert_eq!(
        out,
        "loop=100\nbye 100\nloop=101\nbye 101\nloop=102\nend\nbye 102\n",
        "each iteration's result must be reclaimed when the slot is \
         reused, and the last one at scope exit — three teardowns, \
         not one"
    );
}

/// Argument position. A bare call result there was already owned by
/// the frame (GH #402's unbound-temporary rule — #814's rule covers
/// LITERALS in expression position, which is a different rule and
/// does not reach a call result). The `or` form was not, for the same
/// `Expr::Or` reason, and now is.
#[test]
fn an_argument_position_or_wrapped_factory_result_is_reclaimed() {
    let src = format!(
        "{LIB}
        fn peek(t: Thing) -> Int {{ return t.n; }}
        fn main() {{
            println(\"bare=\", peek(make(7)));
            println(\"wrapped=\", peek(make_f(8) or raise));
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("arg_position", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    assert_eq!(
        out, "bare=7\nwrapped=8\nend\nbye 8\nbye 7\n",
        "an unbound temporary is owned by the frame in either \
         spelling, and lives across the call that consumes it"
    );
}

/// The same defect measured in bytes, in the shape where the wasted
/// storage is LeakSanitizer-visible: a factory result bound inside a
/// locus METHOD's frame. That frame's allocations go to a per-call
/// scratch sub-region, but the factory's own arena is a free-standing
/// `lotus_arena_create_labeled`, so nothing reclaimed it and the
/// `@form(vec)` buffer it grew — freed only on the dissolve path —
/// went with it. This is GH #383's headline shape in the `or`
/// spelling, and it is what `std::process::spawn`'s 216 bytes per
/// call were (the same measurement in `main` reads clean: main's own
/// arena is destroyed at exit, which is why the leak needed a method
/// frame to be seen at all).
///
/// One `#[test]` on purpose: `LOTUS_ASAN` is read by
/// `build_executable` at codegen time and the variable is
/// process-global, so a second test building concurrently in this
/// process would see it.
#[test]
fn a_factory_result_in_a_method_frame_is_leak_clean_under_asan() {
    let src = r#"
        @form(vec)
        locus Buf {
            params { n: Int = 0; }
            capacity { heap data of Float; }
        }

        fn zeros_f(n: Int) -> Buf fallible(String) {
            if n < 0 { fail "negative"; }
            let b = Buf { n: n };
            let mut i = 0;
            while i < n { b.push(1.5); i = i + 1; }
            return b;
        }

        locus Engine {
            params { runs: Int = 0; }
            fn step(n: Int) -> Float {
                let b = zeros_f(n) or Buf { n: 0 };
                let v = b.get(0) or 0.0 - 1.0;
                self.runs = self.runs + 1;
                return v;
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
    let bin = harness::unique_bin("factory_or_asan");
    std::env::set_var("LOTUS_ASAN", "1");
    let built = build_executable(&program, &bin);
    std::env::remove_var("LOTUS_ASAN");
    built.expect("build");
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
    // The values still have to be right. Two of the four earlier
    // attempts on GH #383 were leak-clean and numerically WRONG,
    // which is the state that looks like success under a sanitizer.
    assert!(
        report.contains("t=6") && report.contains("runs=4"),
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
