//! GH #836 — a factory-built locus in a param field is dissolved by
//! its owner.
//!
//! `Router { quick: make(5), tag: 1 }` never dissolved its child. The
//! two halves of the ownership decision had drifted apart: the F.17
//! gate (GH #583, extended to the `or` spelling by GH #793) already
//! kept the enclosing FRAME out of it — "the frame does not reclaim
//! it, the parent does" — but the parent's `__locus_ref_owned_mask`
//! bit, which is what the F.29 cascade reads to tell a child it owns
//! from one an outer scope owns, was only ever set by a LITERAL
//! field init. So the frame stood back, the parent stepped over it,
//! and nothing ran the child's `dissolve()`, its own cascade or its
//! arena destroy.
//!
//! That mask bit now follows the VALUE rather than the spelling: a
//! proven-fresh factory's result transfers into the field exactly as
//! a literal's does, bare or reached through a diverging `or`, and at
//! the param DEFAULT site as well as the call site.
//!
//! "Proven-fresh" is the discriminator GH #383 and GH #402 already
//! decide ownership with, not a new one — `fresh_locus_factories`,
//! the fixpoint over fns whose every return arm is a freshly built
//! value of the declared locus. It is what separates a FACTORY from
//! an ACCESSOR: `pick(x, y, n)` returns a locus its own `let` still
//! owns, and setting the bit for that would dissolve it twice.
//!
//! Two shapes are therefore left exactly as they were, and both are
//! pinned below because they are what this fix must NOT change: that
//! accessor, and `or <substitute>`, where the field holds whichever
//! branch ran and the substitute carries an owner of its own. A
//! missing bit is the old leak; an extra one is a double free, so
//! the rule only claims a field whose value is statically the
//! factory's.
//!
//! A printing `dissolve()` is what makes all of it visible: a missing
//! teardown is silence and a double teardown is the tag twice. The
//! sanitizer case has to run from a locus METHOD frame — measured in
//! `main` it passes vacuously, because main's own arena is destroyed
//! at exit and hides the leak (GH #793's note).

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

/// Every program here answers in milliseconds. The deadline is not
/// about slow machines: a teardown that fires on a value another
/// owner still holds can leave a program spinning rather than
/// exiting, and a bare `Command::output()` would hang a CI job
/// instead of failing it.
const DEADLINE: Duration = Duration::from_secs(60);

fn build_and_run(tag: &str, src: &str) -> (String, String) {
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin(&format!("factory_field_{}", tag));
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

/// A child whose teardown is observable, an owner that holds one in a
/// param field, and the two factories the field is initialised from.
const LIB: &str = r#"
    locus Quick {
        params { n: Int = 0; }
        dissolve() { println("quick ", self.n); }
    }

    locus Router {
        params {
            quick: Quick = Quick { n: 0 };
            tag: Int = 0;
        }
        dissolve() { println("router ", self.tag); }
    }

    fn make(n: Int) -> Quick {
        return Quick { n: n };
    }

    fn make_f(n: Int) -> Quick fallible(String) {
        if n < 0 { fail "negative"; }
        return Quick { n: n };
    }
"#;

/// The headline, with the literal-built child as its control. All
/// three routers used to print their own tag and nothing else; only
/// the literal's child was ever torn down.
#[test]
fn a_factory_built_param_field_dissolves_at_the_owners_teardown() {
    let src = format!(
        "{LIB}
        fn main() {{
            let ctl = Router {{ quick: Quick {{ n: 1 }}, tag: 10 }};
            println(\"ctl=\", ctl.quick.n);
            let a = Router {{ quick: make(2), tag: 20 }};
            println(\"a=\", a.quick.n);
            let b = Router {{ quick: make_f(3) or raise, tag: 30 }};
            println(\"b=\", b.quick.n);
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("both_spellings", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    // Reverse-order flush over the three bindings; within each, the
    // owner's own `dissolve()` body runs before its cascade, so it
    // can still read the child it is about to reclaim.
    assert_eq!(
        out,
        "ctl=1\na=2\nb=3\nend\n\
         router 30\nquick 3\nrouter 20\nquick 2\nrouter 10\nquick 1\n",
        "a factory-built param field must be reclaimed by its owner \
         exactly as a literal-built one is, in both spellings, once \
         each and after the owner's own dissolve body"
    );
}

/// The cascade is recursive (GH #750 / #811), so a factory child
/// whose OWN param field is a factory grandchild has to be reached
/// all the way down. Before the fix the chain stopped at the router:
/// neither `mid` nor `quick` was torn down.
#[test]
fn the_cascade_reaches_a_factory_built_grandchild() {
    let src = r#"
        locus Quick {
            params { n: Int = 0; }
            dissolve() { println("quick ", self.n); }
        }

        locus Mid {
            params {
                quick: Quick = Quick { n: 0 };
                tag: Int = 0;
            }
            dissolve() { println("mid ", self.tag); }
        }

        locus Router {
            params {
                mid: Mid = Mid { tag: 0 };
                tag: Int = 0;
            }
            dissolve() { println("router ", self.tag); }
        }

        fn make_quick(n: Int) -> Quick {
            return Quick { n: n };
        }

        fn make_mid(n: Int) -> Mid {
            return Mid { quick: make_quick(n), tag: n };
        }

        fn main() {
            let r = Router { mid: make_mid(4), tag: 40 };
            println("r=", r.mid.quick.n);
            println("end");
        }
    "#;
    let (out, verdict) = build_and_run("nested", src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    assert_eq!(
        out, "r=4\nend\nrouter 40\nmid 4\nquick 4\n",
        "the owner's cascade must descend through a factory-built \
         child into the factory-built child of ITS param field"
    );
}

/// `or <substitute>` in the same position, on both branches. The
/// rule this fix adds deliberately stays out of it: with a substitute
/// the field holds one of two values decided at run time, so claiming
/// it for the owner unconditionally would put a SECOND owner on a
/// value that already has one. It does not need claiming — a locus
/// literal written there consumes the parent-field flag and sets the
/// same bit the ordinary way, which covers the err branch, while the
/// ok branch's factory result is left to the frame by the F.17 gate
/// and reclaimed by that same bit. One teardown per value, whichever
/// branch ran.
#[test]
fn an_or_substitute_in_a_param_field_tears_down_one_value_per_branch() {
    let src = format!(
        "{LIB}
        fn main() {{
            let ok = Router {{ quick: make_f(2) or Quick {{ n: 99 }}, tag: 20 }};
            println(\"ok=\", ok.quick.n);
            let bad = Router {{ quick: make_f(0 - 1) or Quick {{ n: 98 }}, tag: 30 }};
            println(\"bad=\", bad.quick.n);
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("or_substitute", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    // `99` is never constructed — the substitute is lazy — so the
    // only values to reclaim are the factory's `2` and the
    // substitute's `98`, once each.
    assert_eq!(
        out, "ok=2\nbad=98\nend\nrouter 30\nquick 98\nrouter 20\nquick 2\n",
        "the owner must reclaim exactly the value its field ended up \
         holding, on either branch, and the unused substitute must \
         not be built at all"
    );
}

/// The guard. `pick` hands back a locus it did not build — one of its
/// own arguments — so the `let` that built that locus is still its
/// owner. The bit must stay clear: setting it would dissolve the
/// value once from the router's cascade and once from the binding's
/// scope. Identical output before and after the fix, on purpose.
#[test]
fn an_accessor_returned_handle_is_left_to_its_real_owner() {
    let src = r#"
        locus Quick {
            params { n: Int = 0; }
            dissolve() { println("quick ", self.n); }
        }

        locus Router {
            params {
                quick: Quick = Quick { n: 0 };
                tag: Int = 0;
            }
            dissolve() { println("router ", self.tag); }
        }

        fn pick(a: Quick, b: Quick, which: Int) -> Quick {
            if which == 0 { return a; }
            return b;
        }

        fn main() {
            let x = Quick { n: 5 };
            let y = Quick { n: 6 };
            let r = Router { quick: pick(x, y, 1), tag: 40 };
            println("r=", r.quick.n);
            println("end");
        }
    "#;
    let (out, verdict) = build_and_run("accessor", src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    // `quick 6` appears once, from `y`'s binding — not a second time
    // from the router's cascade, and not instead of it.
    assert_eq!(
        out, "r=6\nend\nrouter 40\nquick 6\nquick 5\n",
        "a call that returns a locus somebody else owns must leave \
         the owner's mask bit clear — exactly one teardown per value"
    );
}

/// A param DEFAULT that is a factory call is the same transfer into
/// the same field, so it takes the same bit. Written with no override
/// at the call site, which is the only way to reach the default-init
/// arm.
#[test]
fn a_factory_param_default_dissolves_with_its_owner() {
    let src = r#"
        locus Quick {
            params { n: Int = 0; }
            dissolve() { println("quick ", self.n); }
        }

        fn make(n: Int) -> Quick {
            return Quick { n: n };
        }

        locus Router {
            params {
                quick: Quick = make(7);
                tag: Int = 0;
            }
            dissolve() { println("router ", self.tag); }
        }

        fn main() {
            let r = Router { tag: 70 };
            println("r=", r.quick.n);
            println("end");
        }
    "#;
    let (out, verdict) = build_and_run("param_default", src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    assert_eq!(
        out, "r=7\nend\nrouter 70\nquick 7\n",
        "a param whose DEFAULT is a factory call must be reclaimed \
         by the owner it defaulted into"
    );
}

/// GH #815's per-iteration reclaim has to reach the child too. The
/// owner's deferred slot is one alloca per SITE, so a site in a loop
/// rewrites it every iteration; the reuse teardown runs the owner's
/// whole spine, cascade included, which is what carries the new bit.
#[test]
fn an_owner_in_a_loop_reclaims_its_factory_child_every_iteration() {
    let src = format!(
        "{LIB}
        @unbounded
        fn main() {{
            let mut i = 0;
            while i < 3 {{
                let r = Router {{ quick: make(i), tag: i }};
                println(\"loop=\", r.quick.n);
                i = i + 1;
            }}
            println(\"end\");
        }}"
    );
    let (out, verdict) = build_and_run("loop", &src);
    assert!(verdict.is_empty(), "{}\n{}", verdict, out);
    assert_eq!(
        out,
        "loop=0\nrouter 0\nquick 0\n\
         loop=1\nrouter 1\nquick 1\n\
         loop=2\nend\nrouter 2\nquick 2\n",
        "each iteration's child must go with that iteration's owner \
         — three teardowns, not one"
    );
}

/// The same defect measured in bytes. The owner is built inside a
/// locus METHOD's frame, which is where the waste is
/// LeakSanitizer-visible: a factory's locus arena is a free-standing
/// `lotus_arena_create_labeled`, and the `@form(vec)` buffer it grew
/// is freed only on the dissolve path, so an unreached child took
/// both with it. Measured in `main` the same program reads clean —
/// main's arena is destroyed at exit (GH #793's note) — which is why
/// the leak needed a method frame to be seen at all.
///
/// The instrumented build is requested through
/// `BuildOptions::asan`, so it is scoped to this build alone
/// (GH #843).
#[test]
fn a_factory_field_in_a_method_frame_is_leak_clean_under_asan() {
    let src = r#"
        @form(vec)
        locus Buf {
            params { n: Int = 0; }
            capacity { heap data of Float; }
        }

        locus Holder {
            params {
                buf: Buf = Buf { n: 0 };
                tag: Int = 0;
            }
        }

        fn zeros(n: Int) -> Buf {
            let b = Buf { n: n };
            let mut i = 0;
            while i < n { b.push(1.5); i = i + 1; }
            return b;
        }

        fn zeros_f(n: Int) -> Buf fallible(String) {
            if n < 0 { fail "negative"; }
            let b = Buf { n: n };
            let mut i = 0;
            while i < n { b.push(2.5); i = i + 1; }
            return b;
        }

        locus Engine {
            params { runs: Int = 0; }
            fn step(n: Int) -> Float {
                let h = Holder { buf: zeros(n), tag: n };
                let g = Holder { buf: zeros_f(n) or raise, tag: n };
                let v = h.buf.get(0) or 0.0 - 1.0;
                let w = g.buf.get(0) or 0.0 - 1.0;
                self.runs = self.runs + 1;
                return v + w;
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
    let bin = harness::unique_bin("factory_field_asan");
    // GH #843: an ASan build is a per-build option, not a
    // process-wide `LOTUS_ASAN` that every concurrent build in this
    // binary would also have picked up. The helper checks the
    // artifact is really instrumented — the assertions below are all
    // negative, so an uninstrumented build passes them vacuously.
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
    // The values still have to be right. A reclaim that fires too
    // early is leak-clean and numerically wrong, which is the state
    // that looks like success under a sanitizer.
    assert!(
        report.contains("t=16") && report.contains("runs=4"),
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

/// GH #895 — the same measurement for an INTERFACE-typed field.
///
/// The field declares a contract (`j: Counter`) and the factory
/// declares the impl (`make_churner() -> Churner`), so the rule
/// above could not fire for it: it asks whether the factory's
/// declared locus IS the field's, and a contract-typed field has no
/// locus to be. The F.17 gate reads an `Interface` field all the
/// same, so the frame already stood back — leaving the child with no
/// owner at all, which is the leak here in bytes: a free-standing
/// locus arena per `Churner`, plus the `@form(vec)` buffer its
/// `Rows` grew, four times over.
///
/// From a METHOD frame for the reason the twin above is: measured in
/// `main` the same program reads clean, because main's arena is
/// destroyed at exit (GH #793's note).
///
/// `LOTUS_NO_CHUNK_POOL=1` on the child as well. The sanitizer
/// cflags default it on (GH #816), and stating it here keeps the
/// negative assertions from being answered by a recycled chunk that
/// still holds its bytes.
#[test]
fn an_interface_factory_field_in_a_method_frame_is_leak_clean_under_asan() {
    let src = r#"
        type Row { v: Int = 0; }

        @form(vec)
        locus Rows { capacity { heap rows of Row; } }

        interface Counter { fn count() -> Int; }

        locus Churner {
            params { rows: Rows = Rows { }; }
            birth() { self.rows.push(Row { v: 3 }); }
            fn count() -> Int { return self.rows.len(); }
        }

        locus Queries {
            params { j: Counter = Churner { }; }
            fn total() -> Int { return self.j.count(); }
        }

        fn make_churner() -> Churner {
            return Churner { };
        }

        fn make_churner_f(n: Int) -> Churner fallible(String) {
            if n < 0 { fail "negative"; }
            return Churner { };
        }

        locus Engine {
            params { runs: Int = 0; }
            fn step() -> Int {
                let q = Queries { j: make_churner() };
                let r = Queries { j: make_churner_f(1) or raise };
                self.runs = self.runs + 1;
                return q.total() + r.total();
            }
        }

        fn main() {
            let e = Engine { };
            let mut t = 0;
            let mut i = 0;
            while i < 4 { t = t + e.step(); i = i + 1; }
            println("t=", t);
            println("runs=", e.runs);
        }
    "#;
    let program = hale_syntax::parse_source(src).expect("parse");
    let bin = harness::unique_bin("factory_iface_field_asan");
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
    // A reclaim that fires too early is leak-clean and wrong, which
    // is the state that looks like success under a sanitizer.
    assert!(
        report.contains("t=8") && report.contains("runs=4"),
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
