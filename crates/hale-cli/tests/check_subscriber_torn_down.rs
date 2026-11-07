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
//! read). Nor at a statement literal inside a `run()` body (a nested
//! block's included: the runtime keeps it alive while the `run()` runs,
//! so it hears), nor at a holder with neither hook whose params field
//! holds a child that has a `run()` (the statement lives while that
//! child's `run()` runs). Until 2026-11-06 the program was silent at the
//! check and at run time (downstream handoff); until 2026-11-07 the
//! advisory fired on the `run()` and holder shapes, which deliver
//! (downstream handoff review).
//!
//! Every advisory case here is paired with what the built program does:
//! a case that fires is shown to deliver nothing, a silent case to
//! deliver (`run`, through `hale run`).
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

/// Build and run `text` as a program (`hale run`): its stdout, the run
/// having exited 0 (the lifecycle shapes here all end on their own).
fn run(tag: &str, text: &str) -> String {
    let root: PathBuf = std::env::temp_dir().join(format!("hale_run_torn_down_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("main.hl");
    std::fs::write(&file, text).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("run")
        .arg(&file)
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    let _ = std::fs::remove_dir_all(&root);
    assert!(out.status.success(), "{tag}: {:?}\n{}", out.status, String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const FINDING: &str = "with no `birth()` or `run()` of its own";

/// What `Empire` prints when the tick reaches it.
const HEARD: &str = "empire 1 tick 7";

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
    let ran = run("birth", &program("birth", "", "", "Empire { empire_id: 1 }; Empire { empire_id: 2 };"));
    assert!(!ran.contains("empire"), "torn down in birth(), so nothing hears:\n{ran}");
}

/// A statement literal in a `run()` body is kept alive while the `run()`
/// runs, in the body itself and in a block nested in it, in a child
/// locus and in the main locus alike: the tick reaches it, and the
/// check says nothing.
#[test]
fn not_in_a_run_body_which_delivers() {
    for (tag, build) in [("run_body", "Empire { empire_id: 1 };"), ("run_body_block", "if true { Empire { empire_id: 1 }; }")] {
        let text = program("run", "", "", build);
        let (ok, out) = check(tag, &text);
        assert!(ok, "{out}");
        assert_eq!(out.matches(FINDING).count(), 0, "{tag}: a literal in run() lives while run() runs:\n{out}");
        let ran = run(tag, &text);
        assert!(ran.contains(HEARD), "{tag}: the tick reached it:\n{ran}");
    }
    let text = "type Tick { n: Int = 0; }
topic Ticks { payload: Tick; subject: \"t\"; }
locus Empire {
    params { empire_id: Int = 0; }
    bus { subscribe Ticks as on_tick; }
    fn on_tick(t: Tick) { println(\"empire \", self.empire_id, \" tick \", t.n); }
}
main locus App {
    bus { publish Ticks; }
    run() { Empire { empire_id: 1 }; Ticks <- Tick { n: 7 }; yield; }
}
fn main() { App { }; }
";
    let (ok, out) = check("main_run", text);
    assert!(ok, "{out}");
    assert_eq!(out.matches(FINDING).count(), 0, "the main locus's run():\n{out}");
    let ran = run("main_run", text);
    assert!(ran.contains(HEARD), "{ran}");
}

/// `Rig`, with neither hook, holds a subscriber and a publisher child
/// whose `run()` publishes: built as a statement in a free fn (or as the
/// main locus, pinned), the statement lives while the child's `run()`
/// runs and the subscriber hears. With no running child beside the
/// subscriber, the holder is torn down at the statement and the
/// publisher built after it reaches nothing: that one is reported.
fn rig(main: bool, pub_inside: bool) -> String {
    let fields = if pub_inside { "s: Store = Store { }; p: Pub = Pub { };" } else { "s: Store = Store { };" };
    let (head, placement, build) = if main {
        ("main locus Rig", "placement { s: pinned; p: pinned; }", "Rig { };")
    } else if pub_inside {
        ("locus Rig", "", "Rig { };")
    } else {
        ("locus Rig", "", "Rig { }; Pub { };")
    };
    format!(
        "type Rec {{ name: String = \"\"; }}
topic Recs {{ payload: Rec; subject: \"recs\"; }}
locus Store {{
    bus {{ subscribe Recs as on_rec; }}
    fn on_rec(r: Rec) {{ println(\"got \" + r.name); }}
}}
locus Pub {{
    bus {{ publish Recs; }}
    run() {{
        let mut i = 0;
        while i < 3 {{ Recs <- Rec {{ name: \"rec\" + to_string(i) }}; i = i + 1; }}
        std::time::sleep(50ms);
    }}
}}
{head} {{
    params {{ {fields} }}
    {placement}
}}
fn build() {{ {build} }}
fn main() {{ build(); println(\"after\"); }}
"
    )
}

#[test]
fn not_at_a_holder_whose_child_runs_which_delivers() {
    for (tag, main) in [("rig_inline", false), ("rig_pinned", true)] {
        let text = rig(main, true);
        let (ok, out) = check(tag, &text);
        assert!(ok, "{out}");
        assert_eq!(out.matches(FINDING).count(), 0, "{tag}: the holder lives while its child's run() runs:\n{out}");
        let ran = run(tag, &text);
        for i in 0..3 {
            assert!(ran.contains(&format!("got rec{i}")), "{tag}: rec{i} reached the held subscriber:\n{ran}");
        }
    }
}

#[test]
fn a_holder_with_no_running_child_is_warned_and_hears_nothing() {
    let text = rig(false, false);
    let (ok, out) = check("rig_idle", &text);
    assert!(ok, "{out}");
    assert_eq!(out.matches(FINDING).count(), 1, "{out}");
    assert!(out.contains("`Rig` holds a bus subscriber for its whole life"), "{out}");
    assert!(out.contains("as a statement of `build`"), "{out}");
    let ran = run("rig_idle", &text);
    assert!(!ran.contains("got "), "torn down at its statement, before the publisher ran:\n{ran}");
    assert!(ran.contains("after"), "{ran}");
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
    let ran = run("let", src);
    assert_eq!(ran.trim(), "7", "the local heard the tick:\n{ran}");
}

#[test]
fn in_a_method_and_in_a_free_fn() {
    let text = program("method", "", "", "Empire { empire_id: 1 };");
    let (ok, out) = check("method", &text);
    assert!(ok, "{out}");
    assert!(out.contains("as a statement of `Parent.setup`"), "{out}");
    assert!(!run("method", &text).contains("empire"), "torn down in the method, so nothing hears");
    let text = program("free", "", "", "Empire { empire_id: 1 };");
    let (ok, out) = check("free", &text);
    assert!(ok, "{out}");
    assert!(out.contains("as a statement of `setup`"), "{out}");
    assert!(out.contains("`accept` it in a locus that builds it"), "{out}");
    assert!(!run("free", &text).contains("empire"), "torn down in the free fn, so nothing hears");
}

#[test]
fn not_when_the_parent_accepts_it() {
    let text = program("birth", "accept(e: Empire) { }", "", "Empire { empire_id: 1 }; Empire { empire_id: 2 };");
    let (ok, out) = check("accept", &text);
    assert!(ok, "{out}");
    assert_eq!(out.matches(FINDING).count(), 0, "accepted, so owned and heard:\n{out}");
    let ran = run("accept", &text);
    assert!(ran.contains(HEARD) && ran.contains("empire 2 tick 7"), "both accepted children heard:\n{ran}");
}

#[test]
fn not_when_the_subscriber_declares_a_run() {
    let text = program("birth", "", "run() { println(\"empire \", self.empire_id, \" ran\"); }", "Empire { empire_id: 1 };");
    let (ok, out) = check("declares_run", &text);
    assert!(ok, "{out}");
    assert_eq!(out.matches(FINDING).count(), 0, "a statement-position locus with a run() runs it:\n{out}");
    assert!(run("declares_run", &text).contains("empire 1 ran"), "the build did its own work");
}

#[test]
fn not_when_the_subscriber_declares_a_birth() {
    let text = program("birth", "", "birth() { println(\"born\"); }", "Empire { empire_id: 1 };");
    let (ok, out) = check("birth_hook", &text);
    assert!(ok, "{out}");
    assert_eq!(out.matches(FINDING).count(), 0, "a fire-and-forget literal does its work in birth():\n{out}");
    assert!(run("birth_hook", &text).contains("born"), "the build did its own work");
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
    let text = program("birth", "params { e: Empire = Empire { empire_id: 1 }; }", "", "");
    let (ok, out) = check("field", &text);
    assert!(ok, "{out}");
    assert_eq!(out.matches(FINDING).count(), 0, "a params field keeps it for the parent's life:\n{out}");
    assert!(run("field", &text).contains(HEARD), "the held child heard the tick");
}
