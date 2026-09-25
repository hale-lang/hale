//! `restart_in_place(c)` keeps the params the instance was built with.
//!
//! Both restart paths — the birth-epoch rerun in `__birth_closures` and
//! `__restart_<L>` after a failed `run()` — used to re-store every param
//! from its DECLARED default, re-evaluating the default expression. A
//! child built as `Worker { tag: "from-literal" }` came back with the
//! declared tag, and a default that builds a locus (`t: Tries = Tries
//! { }`) built a fresh one each restart, orphaning the old one — so the
//! birth-epoch child below, whose closure counts the `Tries` it pushed
//! into, could never pass and failed until its bound quarantined it.
//!
//! spec/semantics.md § "restart_in_place(child)": params are settled
//! once, from the literal; a restart never re-evaluates a default. The
//! instance keeps a copy of its params as built and the restart restores
//! it; a param holding a locus keeps its child.
//!
//! The restored values are heap strings, and the run is under ASan with
//! the chunk pool off (a restored copy must be the field's own block).

use std::process::Command;

#[path = "support/harness.rs"]
mod harness;

const SRC: &str = r#"
@form(vec)
locus Tries { capacity { heap items of Int; } }
locus Worker {
    params { tag: String = std::str::upper("declared"); hits: Int = 0; note: String = ""; t: Tries = Tries { }; }
    closure fuse { captures: hits; epoch inline; }
    run() {
        self.t.push(1);
        self.hits = self.hits + 1;
        self.note = self.note + std::str::upper("r");
        if self.t.len() < 2 { violate fuse; }
    }
}
locus Born {
    params { tag: String = std::str::upper("declared"); t: Tries = Tries { }; }
    closure settled { self.t.len() ~~ 2 within 0; epoch birth; }
    birth() { self.t.push(1); }
}
main locus App {
    params { w: Worker = Worker { tag: std::str::upper("from-literal") }; s: Sup = Sup { }; seen: String = ""; }
    on_failure(c: Worker, err: ClosureViolation) { self.seen = self.seen + c.tag + ":" + to_string(c.hits) + ":" + c.note + ";"; restart_in_place(c) for 3; }
    run() {
        println("w tag=" + self.w.tag + " hits=" + to_string(self.w.hits) + " note=" + self.w.note + " tries=" + to_string(self.w.t.len()) + " seen=" + self.seen);
        println("born tag=" + self.s.b.tag + " tries=" + to_string(self.s.b.t.len()) + " fired=" + to_string(self.s.fired));
    }
}
locus Sup {
    params { fired: Int = 0; b: Born = Born { tag: std::str::upper("born-literal") }; }
    on_failure(c: Born, err: ClosureViolation) { self.fired = self.fired + 1; restart_in_place(c) for 3; }
}
fn main() { App { }; }
"#;

#[test]
fn restart_in_place_restores_the_params_as_built_on_both_paths() {
    let program = hale_syntax::parse_source(SRC).expect("parse");
    let bin = harness::unique_bin("hale_restart_in_place_params");
    harness::build_asan(&program, &bin);
    let out = Command::new(&bin)
        .env("LOTUS_NO_CHUNK_POOL", "1")
        .output()
        .expect("run");
    let _ = std::fs::remove_file(&bin);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "non-zero {:?}; stdout: {stdout}; stderr: {stderr}", out.status);
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        [
            // run path: the literal's tag, hits/note back to their built
            // values then one more run, the Tries child kept (2 pushes)
            "w tag=FROM-LITERAL hits=1 note=R tries=2 seen=FROM-LITERAL:1:R;",
            // birth path: the literal's tag, the same Tries child, one
            // failure then a birth that passes
            "born tag=BORN-LITERAL tries=2 fired=1",
        ],
        "stderr: {stderr}"
    );
}
