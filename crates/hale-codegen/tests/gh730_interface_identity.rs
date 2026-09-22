//! GH #730, first half — an interface VALUE into a field of the same
//! interface is identity, and the holder borrows the impl.
//!
//! `Attempt { performer: self.performer }` with both sides `Performer`
//! was refused as "type `Performer` cannot satisfy interface
//! `Performer`" (dna/FRICTION.md F.3). Now the child stores its own
//! fat pair and owns nothing: the impl dissolves once, with its real
//! owner, after every child that borrowed it. A different interface
//! is still refused by the checker.

use std::process::Command;

use hale_codegen::build_executable;

#[path = "support/harness.rs"]
mod harness;

fn build_and_run(name: &str, source: &str) -> (bool, String) {
    let program = hale_syntax::parse_source(source).expect("parse");
    assert!(
        !hale_types::check_program(&program).iter().any(|d| d.is_error()),
        "the checker admits it: {:?}",
        hale_types::check_program(&program)
    );
    let bin = harness::unique_bin(&format!("gh730_{}", name));
    build_executable(&program, &bin).expect("build");
    let output = Command::new(&bin).output().expect("run");
    let _ = std::fs::remove_file(&bin);
    (output.status.success(), String::from_utf8_lossy(&output.stdout).to_string())
}

const PERFORMER: &str = r#"
interface Performer { fn perform(x: Int) -> Int; fn name() -> String; }
locus Doubler {
    params { calls: Int = 0; }
    fn perform(x: Int) -> Int { self.calls = self.calls + 1; return x * 2; }
    fn name() -> String { return "doubler"; }
    dissolve() { println("doubler dissolved after ", self.calls, " calls"); }
}
"#;

/// The F.3 reproducer: two accepted flow children share their
/// parent's performer.
#[test]
fn a_parent_hands_its_interface_field_to_accepted_children() {
    let src = format!(
        "{PERFORMER}
locus Attempt {{
    params {{ performer: Performer; out: Int = 0; }}
    contract {{ expose out: Int; }}
    run() {{ self.out = self.performer.perform(21); }}
}}
locus Work {{
    params {{ performer: Performer = Doubler {{ }}; total: Int = 0; }}
    accept(a: Attempt) {{ }}
    release(a: Attempt) {{ self.total = self.total + a.out; }}
    run() {{
        Attempt {{ performer: self.performer }};
        Attempt {{ performer: self.performer }};
        println(\"total=\", self.total, \" name=\", self.performer.name());
    }}
}}
fn main() {{ Work {{ }}; }}
"
    );
    let (ok, out) = build_and_run("accepted", &src);
    assert!(ok, "{out}");
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        vec!["total=84 name=doubler", "doubler dissolved after 2 calls"],
        "both children dispatched through the one impl, which its owner dissolved once, last: {out:?}"
    );
}

/// A field of `self` into a field-owned child built in `birth`, the
/// assembly's shape: the child borrows, the parent owns.
#[test]
fn a_field_owned_child_borrows_its_parents_interface_field() {
    let src = format!(
        "{PERFORMER}
locus Runtime {{
    params {{ performer: Performer = Doubler {{ }}; }}
    fn go() -> Int {{ return self.performer.perform(5); }}
}}
locus Holder {{
    params {{ performer: Performer = Doubler {{ }}; rt: Runtime = Runtime {{ }}; }}
    birth() {{ self.rt = Runtime {{ performer: self.performer }}; }}
    fn go() -> Int {{ return self.rt.go() + self.performer.perform(1); }}
}}
fn main() {{ let h = Holder {{ }}; println(h.go()); }}
"
    );
    let (ok, out) = build_and_run("field_owned", &src);
    assert!(ok, "{out}");
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        vec![
            // the runtime's own default, reclaimed when birth replaced the runtime (break-before-make)
            "doubler dissolved after 0 calls",
            "12",
            // the holder's impl, borrowed by the runtime: dissolved once, with the holder, after both used it
            "doubler dissolved after 2 calls",
        ],
        "the replaced runtime's default went at the replacement; the borrowed impl once, with its owner, last: {out:?}"
    );
}

/// A value of a different interface into the slot is still refused.
#[test]
fn a_different_interface_is_still_refused() {
    let src = format!(
        "{PERFORMER}
interface Namer {{ fn name() -> String; }}
locus Attempt {{ params {{ performer: Performer; }} run() {{ }} }}
locus Work {{
    params {{ namer: Namer = Doubler {{ }}; }}
    accept(a: Attempt) {{ }}
    release(a: Attempt) {{ }}
    run() {{ Attempt {{ performer: self.namer }}; }}
}}
fn main() {{ Work {{ }}; }}
"
    );
    let program = hale_syntax::parse_source(&src).expect("parse");
    let diags = hale_types::check_program(&program);
    assert!(
        diags.iter().any(|d| d.is_error() && d.message.contains("cannot satisfy interface")),
        "a Namer into a Performer slot is refused: {diags:?}"
    );
}
