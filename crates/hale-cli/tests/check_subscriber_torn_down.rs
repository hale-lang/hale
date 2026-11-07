//! The "Subscriber torn down at its statement" advisory
//! (spec/verification.md): a bus subscriber built as a bare statement,
//! with no `birth()` or `run()` of its own and no `accept` owning it, is
//! torn down inside the statement, its subscription with it, so no
//! delivery can reach it and the program is silent by design
//! (spec/semantics.md § "Locus instantiation"). `hale check` warns at the
//! literal, in any body; not when the enclosing locus accepts the type,
//! not when the subscriber declares a `run()` or a `birth()` (the build
//! does its own work), not when it is held in a field, and not when it
//! is let-bound: a local lives for its scope and hears across a park
//! point there (the reviewer's shape: built, published to, `yield`, then
//! read). Until 2026-11-06 the program was silent at the check and at
//! run time (downstream handoff).
//!
//! The programs are escaped string literals rather than raw strings:
//! `hale-corpus` harvests raw-string literals out of test files, and these
//! are deliberately diagnostic.

use std::path::{Path, PathBuf};
use std::process::Command;

/// `Parent`, with `extra` among its members, building `Empire` (declared
/// with `empire_extra` among its members) as `build` writes it, in the
/// body `where` names (`birth`, `run`, a method called from `run`, or a
/// free fn called from `run`).
fn program(where_: &str, extra: &str, empire_extra: &str, build: &str) -> String {
    let (members, helper) = match where_ {
        "birth" => (format!("birth() {{ {build} }}\n    run() {{ Ticks <- Tick {{ n: 7 }}; yield; }}"), String::new()),
        "run" => (format!("run() {{ {build} Ticks <- Tick {{ n: 7 }}; yield; }}"), String::new()),
        "method" => (
            format!("fn setup() {{ {build} }}\n    run() {{ self.setup(); Ticks <- Tick {{ n: 7 }}; yield; }}"),
            String::new(),
        ),
        "free" => (
            "run() { setup(); Ticks <- Tick { n: 7 }; yield; }".to_string(),
            format!("fn setup() {{ {build} }}\n"),
        ),
        other => panic!("where: {other}"),
    };
    format!(
        "type Tick {{ n: Int = 0; }}
topic Ticks {{ payload: Tick; subject: \"t\"; }}
locus Empire {{
    params {{ empire_id: Int = 0; }}
    bus {{ subscribe Ticks as on_tick; }}
    fn on_tick(t: Tick) {{ println(\"empire \", self.empire_id, \" tick \", t.n); }}
    {empire_extra}
}}
{helper}locus Parent {{
    {extra}
    bus {{ publish Ticks; }}
    {members}
}}
fn main() {{ let p = Parent {{ }}; }}
"
    )
}

fn check(tag: &str, text: &str) -> (bool, String) {
    let root: PathBuf =
        std::env::temp_dir().join(format!("hale_check_torn_down_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&root);
    let seed = root.join("app");
    std::fs::create_dir_all(&seed).unwrap();
    std::fs::write(seed.join("main.hl"), text).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(&seed)
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    let _ = std::fs::remove_dir_all(&root);
    (out.status.success(), format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)))
}

const FINDING: &str = "with no `birth()` or `run()` of its own";

#[test]
fn statement_built_subscribers_in_birth_are_warned() {
    let (ok, out) = check("birth", &program("birth", "", "", "Empire { empire_id: 1 }; Empire { empire_id: 2 };"));
    assert!(ok, "a warning, not an error:\n{out}");
    assert_eq!(out.matches(FINDING).count(), 2, "one finding per literal:\n{out}");
    assert!(
        out.contains("`Empire` subscribes on the bus, and this literal builds it as a statement of `Parent.birth` with no `birth()` or `run()` of its own: it is torn down at the statement's end, its subscription with it, so no delivery can reach it."),
        "names the locus and the call:\n{out}"
    );
    assert!(out.contains("`accept` it in `Parent` (`accept(c: Empire) { }`), or hold it in a field."), "{out}");
    assert!(out.contains("warning"), "{out}");
}

#[test]
fn not_when_let_bound_and_read_after_a_park_point() {
    let src = "type Tick { n: Int = 0; }
topic Ticks { payload: Tick; subject: \"t\"; }
locus Sink {
    params { last: Int = 0; }
    bus { subscribe Ticks as on_tick; }
    fn on_tick(t: Tick) { self.last = t.n; }
    fn read() -> Int { return self.last; }
}
locus Parent {
    bus { publish Ticks; }
    run() { let sink = Sink { }; Ticks <- Tick { n: 7 }; yield; println(sink.read()); }
}
fn main() { let p = Parent { }; }
";
    let (ok, out) = check("let", src);
    assert!(ok, "{out}");
    assert_eq!(out.matches(FINDING).count(), 0, "a local lives for its scope and hears across the yield:\n{out}");
}

#[test]
fn in_a_method_and_in_a_free_fn() {
    let (ok, out) = check("method", &program("method", "", "", "Empire { empire_id: 1 };"));
    assert!(ok, "{out}");
    assert!(out.contains("as a statement of `Parent.setup`"), "{out}");
    let (ok, out) = check("free", &program("free", "", "", "Empire { empire_id: 1 };"));
    assert!(ok, "{out}");
    assert!(out.contains("as a statement of `setup`"), "{out}");
    assert!(out.contains("`accept` it in a locus that builds it"), "{out}");
}

#[test]
fn not_when_the_parent_accepts_it() {
    let (ok, out) = check("accept", &program("birth", "accept(e: Empire) { }", "", "Empire { empire_id: 1 }; Empire { empire_id: 2 };"));
    assert!(ok, "{out}");
    assert_eq!(out.matches(FINDING).count(), 0, "accepted, so owned and heard:\n{out}");
}

#[test]
fn not_when_the_subscriber_declares_a_run() {
    let (ok, out) = check("run", &program("birth", "", "run() { std::time::sleep(5ms); }", "Empire { empire_id: 1 };"));
    assert!(ok, "{out}");
    assert_eq!(out.matches(FINDING).count(), 0, "a statement-position locus with a run() runs it:\n{out}");
}

#[test]
fn not_when_the_subscriber_declares_a_birth() {
    let (ok, out) = check("birth_hook", &program("birth", "", "birth() { println(\"born\"); }", "Empire { empire_id: 1 };"));
    assert!(ok, "{out}");
    assert_eq!(out.matches(FINDING).count(), 0, "a fire-and-forget literal does its work in birth():\n{out}");
}

#[test]
fn not_in_the_free_fn_main() {
    let text = r#"
type Tick { n: Int = 0; }
topic Ticks { payload: Tick; subject: "t"; }
locus Empire {
    bus { subscribe Ticks as on_tick; }
    fn on_tick(t: Tick) { println("tick ", t.n); }
}
fn main() { Empire { }; }
"#;
    let (ok, out) = check("fn_main", text);
    assert!(ok, "{out}");
    assert_eq!(out.matches(FINDING).count(), 0, "the root built by fn main has no locus to accept it:\n{out}");
}

#[test]
fn not_when_held_in_a_field() {
    let (ok, out) = check("field", &program("birth", "params { e: Empire = Empire { empire_id: 1 }; }", "", ""));
    assert!(ok, "{out}");
    assert_eq!(out.matches(FINDING).count(), 0, "a params field keeps it for the parent's life:\n{out}");
}
