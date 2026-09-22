//! GH #967 — a handle stored into an `interface`-typed field is a
//! BORROW, at assignment exactly as at initialisation (F.39: a name in
//! a field initialiser is `Owner::Borrowed`).
//!
//! Before this, `self.journal = j` on an interface-typed field was a
//! plain value store: the field's F.29 owned bit and its GH #871
//! reclaim slot kept describing the DEFAULT the field was built with,
//! so the holder's cascade ran the default's `__reclaim_<Impl>` on
//! whatever the field pointed at — the impl somebody else owned — and
//! the default itself was never reclaimed. An assembly handed a
//! journal reclaimed it when it died: the next assembly over the same
//! journal read freed memory, and a let-bound journal was reclaimed a
//! second time by its binding (a segfault at exit).
//!
//! The oracle is the `dissolve()` hook printing: the default is
//! dissolved at the store (break-before-make, as WS1#4 does for a
//! literal), the borrowed impl exactly once, by its own owner, after
//! every use. Under `LOTUS_ASAN=1` the same programs are the
//! use-after-free oracle.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, source: &str) -> (bool, String) {
    let program = hale_syntax::parse_source(source).expect("parse");
    let bin = harness::unique_bin(&format!("gh967_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).to_string(),
    )
}

const TYPES: &str = r#"
interface J { fn name() -> String; }
locus Mem {
    params { a: String = "default"; }
    fn name() -> String { return self.a; }
    dissolve() { println("dissolve " + self.a); }
}
locus Rt {
    params { journal: J = Mem { }; }
    fn adopt(j: J) { self.journal = j; }
    fn peek() -> String { return self.journal.name(); }
}
"#;

fn lines(out: &str) -> Vec<&str> {
    out.lines().collect()
}

#[test]
fn a_handle_by_parameter_is_borrowed_and_the_default_reclaimed() {
    let src = format!(
        "{TYPES}
fn f(j: Mem) {{ let r = Rt {{ }}; r.adopt(j); println(r.peek()); }}
fn main() {{ let j = Mem {{ a: \"outer\" }}; f(j); f(j); println(j.name()); }}
"
    );
    let (ok, out) = build_and_run("param", &src);
    assert!(ok, "exit status: {out}");
    assert_eq!(
        lines(&out),
        vec![
            "dissolve default",
            "outer",
            "dissolve default",
            "outer",
            "outer",
            "dissolve outer",
        ],
        "the default is reclaimed at the store, the borrowed journal once by its binding, after every use: {out:?}"
    );
}

#[test]
fn a_handle_from_a_field_is_borrowed() {
    let src = format!(
        "{TYPES}
main locus App {{
    params {{ j: Mem = Mem {{ a: \"outer\" }}; }}
    fn f() {{ let r = Rt {{ }}; r.adopt(self.j); println(r.peek()); }}
    run() {{ self.f(); self.f(); println(self.j.name()); }}
}}
fn main() {{ App {{ }}; }}
"
    );
    let (ok, out) = build_and_run("field", &src);
    assert!(ok, "exit status: {out}");
    assert_eq!(
        lines(&out),
        vec![
            "dissolve default",
            "outer",
            "dissolve default",
            "outer",
            "outer",
            "dissolve outer",
        ],
        "{out:?}"
    );
}

#[test]
fn a_handle_from_a_field_through_a_parameter_is_borrowed() {
    let src = format!(
        "{TYPES}
main locus App {{
    params {{ j: Mem = Mem {{ a: \"outer\" }}; }}
    fn f(x: Mem) {{ let r = Rt {{ }}; r.adopt(x); println(r.peek()); }}
    run() {{ self.f(self.j); self.f(self.j); println(self.j.name()); }}
}}
fn main() {{ App {{ }}; }}
"
    );
    let (ok, out) = build_and_run("field_param", &src);
    assert!(ok, "exit status: {out}");
    assert_eq!(
        lines(&out),
        vec![
            "dissolve default",
            "outer",
            "dissolve default",
            "outer",
            "outer",
            "dissolve outer",
        ],
        "{out:?}"
    );
}

/// The crash of the issue: a let-bound routed journal handed to a
/// holder whose birth replaces a child and hands the journal on. The
/// holder's cascade used to reclaim the routed journal through the
/// child's slot, and main's flush reclaimed it again.
#[test]
fn a_let_bound_journal_handed_on_at_birth_is_reclaimed_once_by_its_binding() {
    let src = format!(
        "{TYPES}
locus Routed {{
    params {{ record: J = Mem {{ }}; }}
    fn name() -> String {{ return \"routed:\" + self.record.name(); }}
}}
locus Holder {{
    params {{ journal: J = Mem {{ }}; rt: Rt = Rt {{ }}; }}
    birth() {{ self.rt = Rt {{ }}; self.rt.adopt(self.journal); }}
    fn peek() -> String {{ return self.rt.peek(); }}
}}
fn main() {{
    let record = Mem {{ a: \"rec\" }};
    let journal = Routed {{ record: record }};
    let owner = Holder {{ journal: journal }};
    println(owner.peek());
}}
"
    );
    let (ok, out) = build_and_run("let_bound", &src);
    assert!(
        ok,
        "the program exits cleanly (it used to segfault at exit): {out}"
    );
    assert_eq!(
        lines(&out),
        vec![
            "dissolve default",
            "dissolve default",
            "routed:rec",
            "dissolve rec",
        ],
        "the replaced child's default and the new child's default are reclaimed; the record once, by its binding: {out:?}"
    );
}

/// A field that already holds a borrow releases nothing when it is
/// assigned again: the first handle stays its owner's.
#[test]
fn a_second_handle_releases_nothing_of_the_first() {
    let src = format!(
        "{TYPES}
fn main() {{
    let one = Mem {{ a: \"one\" }};
    let two = Mem {{ a: \"two\" }};
    let r = Rt {{ }};
    r.adopt(one);
    r.adopt(two);
    println(r.peek() + \" \" + one.name() + \" \" + two.name());
}}
"
    );
    let (ok, out) = build_and_run("rebind", &src);
    assert!(ok, "exit status: {out}");
    let got = lines(&out);
    assert_eq!(got[0], "dissolve default", "{out:?}");
    assert_eq!(got[1], "two one two", "{out:?}");
    let mut tail: Vec<&str> = got[2..].to_vec();
    tail.sort();
    assert_eq!(
        tail,
        vec!["dissolve one", "dissolve two"],
        "each handle dissolved exactly once, by its own binding: {out:?}"
    );
}
