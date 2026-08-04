//! A locus `let`-bound in a locus method must dissolve even when the
//! method exits via `return` (GH #383, found while investigating the
//! factory-return leak — this is the separable half).
//!
//! `lower_return`'s method-with-scratch arms destroyed the scratch and
//! returned without ever flushing the deferred-dissolve frame;
//! `locus/method.rs`'s terminated-body arm then popped that frame with
//! a bare `let _ = …pop();`. Only a fall-through exit (`BlockEnd::Open`)
//! ever flushed. So this leaked, silently, with no diagnostic:
//!
//! ```hale,fragment
//! fn step(i: Int) -> Int {
//!     let w = Watcher { id: i };   // subscribes, opens an fd, …
//!     return i * 2;                // ← w never dissolved
//! }
//! ```
//!
//! Nothing about factories is involved; a plain locus literal leaks
//! the same way. Verified against the v0.13.0 release binary: the
//! `dissolve()` body never runs on either return path.
//!
//! ## Why the flush must precede the scratch destroy
//!
//! A dissolve is a method call, and every call site publishes the
//! caller-arena TLS before invoking. Flushing *after*
//! `close_method_scratch()` therefore publishes a pointer into
//! already-freed scratch, which reproduces the #375/#381
//! use-after-free signature exactly (observed while developing this).
//! The frame's CONTENTS are saved and restored around the flush for
//! the same reason the scratch state is: the flush pops, and a body
//! with N returns must emit dissolves on every path while the
//! epilogue's single pop still balances.

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
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        out.status,
    )
}

const SRC: &str = r#"
    locus Watcher {
        params { id: Int = 0; }
        birth()    { println("born ", self.id); }
        dissolve() { println("dissolved ", self.id); }
    }

    locus Engine {
        params { n: Int = 0; }

        // value return
        fn step(i: Int) -> Int {
            let w = Watcher { id: i };
            self.n = self.n + 1;
            return i * 2;
        }

        // void return
        fn tick(i: Int) {
            let w = Watcher { id: 100 + i };
            if i >= 0 { return; }
        }

        // several returns in one body: every path must dissolve,
        // and the scratch must still be destroyed exactly once
        fn pick(i: Int) -> Int {
            let w = Watcher { id: 200 + i };
            if i == 0 { return 10; }
            if i == 1 { return 20; }
            return 30;
        }
    }

    fn main() {
        let e = Engine { };
        print("s="); println(e.step(1));
        e.tick(1);
        print("p="); println(e.pick(1));
        print("n="); println(e.n);
        println("done");
    }
"#;

#[test]
fn a_let_bound_locus_dissolves_on_every_return_path() {
    let (out, st) = run("method_return_dissolve", SRC);
    assert!(st.success(), "non-zero exit: {:?}\n{}", st, out);

    for id in ["1", "101", "201"] {
        assert!(
            out.contains(&format!("born {}", id)),
            "watcher {} must be born: {:?}",
            id,
            out
        );
        assert!(
            out.contains(&format!("dissolved {}", id)),
            "watcher {} must dissolve despite the method returning \
             (GH #383): {:?}",
            id,
            out
        );
    }
    // …and the methods' own work still happened. The birth /
    // dissolve lines interleave between `print("s=")` and its value
    // (the callee runs after the label is written), so match the
    // values on their own lines rather than as `s=2`.
    let lines: Vec<&str> = out.lines().map(|l| l.trim()).collect();
    assert!(lines.contains(&"2"), "step returned 2: {:?}", out);
    assert!(lines.contains(&"20"), "pick returned 20: {:?}", out);
    assert!(out.contains("n=1"), "got: {:?}", out);
    assert!(out.contains("done"), "got: {:?}", out);
}

/// Ordering guard: the dissolve runs BEFORE the method's scratch is
/// destroyed. If that inverts, the dissolve's call site publishes the
/// caller-arena TLS into freed memory — the #375/#381 UAF. A body that
/// allocates in scratch on both sides of the binding gives the
/// regression somewhere to land.
#[test]
fn dissolve_ordering_does_not_strand_the_caller_arena() {
    const ORDER_SRC: &str = r#"
        locus Note {
            params { tag: String = ""; }
            dissolve() { println("gone ", self.tag); }
        }

        locus Runner {
            params { seen: String = ""; }
            fn work(i: Int) -> String {
                let pre = "pre-" + to_string(i);
                let n = Note { tag: pre };
                let post = pre + "-post";
                self.seen = post;
                return post;
            }
        }

        fn main() {
            let r = Runner { };
            let mut i = 0;
            while i < 3 {
                let s = r.work(i);
                println(s);
                i = i + 1;
            }
            println(r.seen);
        }
    "#;
    let (out, st) = run("method_return_dissolve_order", ORDER_SRC);
    assert!(st.success(), "non-zero exit: {:?}\n{}", st, out);
    for i in 0..3 {
        assert!(
            out.contains(&format!("gone pre-{}", i)),
            "note {} must dissolve: {:?}",
            i,
            out
        );
        assert!(
            out.contains(&format!("pre-{}-post", i)),
            "the returned String must survive the dissolve: {:?}",
            out
        );
    }
}
