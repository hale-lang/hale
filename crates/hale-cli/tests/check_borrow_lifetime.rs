//! GH #730, second half — a borrow must outlive its holder.
//!
//! A handle stored by name into a locus-carrying field is borrowed,
//! never the holder's to reclaim, so the frame, dispatch or binding
//! that owns it has to last longer than the holder. `hale check`
//! refuses the shapes where it cannot, from position alone, and names
//! the witness call when a parameter carries the handle in. A borrow
//! the holder reads only in `birth()` is birth-scoped and sound. Beside
//! it, the GH #737 notice for a subscription key assigned in `birth()`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn check(src: &str, tag: &str) -> (bool, String) {
    let d: PathBuf = std::env::temp_dir()
        .join(format!("hale_borrow_lifetime_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let f = d.join("main.hl");
    std::fs::write(&f, src).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["check", &f.to_string_lossy()])
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&d);
    (out.status.success(), text)
}

const RULE: &str = "would hold a borrow that does not outlive it";

const LOCAL_INTO_ACCEPTED: &str = "interface Performer { fn perform(x: Int) -> Int; }\nlocus Doubler { fn perform(x: Int) -> Int { return x * 2; } }\nlocus Attempt { params { performer: Performer; } run() { let x = self.performer.perform(1); } }\nlocus Rt { params { performer: Performer = Doubler { }; } fn go() -> Int { return self.performer.perform(2); } }\nlocus Work {\n    params { performer: Performer = Doubler { }; }\n    accept(a: Attempt) { }\n    release(a: Attempt) { }\n    run() { let d = Doubler { }; Attempt { performer: d }; }\n}\nfn main() { Work { }; }\n";
const SELF_FIELD_INTO_ACCEPTED: &str = "interface Performer { fn perform(x: Int) -> Int; }\nlocus Doubler { fn perform(x: Int) -> Int { return x * 2; } }\nlocus Attempt { params { performer: Performer; } run() { let x = self.performer.perform(1); } }\nlocus Rt { params { performer: Performer = Doubler { }; } fn go() -> Int { return self.performer.perform(2); } }\nlocus Work {\n    params { performer: Performer = Doubler { }; }\n    accept(a: Attempt) { }\n    release(a: Attempt) { }\n    run() { Attempt { performer: self.performer }; }\n}\nfn main() { Work { }; }\n";
const LOCAL_INTO_SELF_FIELD: &str = "interface Performer { fn perform(x: Int) -> Int; }\nlocus Doubler { fn perform(x: Int) -> Int { return x * 2; } }\nlocus Attempt { params { performer: Performer; } run() { let x = self.performer.perform(1); } }\nlocus Rt { params { performer: Performer = Doubler { }; } fn go() -> Int { return self.performer.perform(2); } }\nlocus Work {\n    params { rt: Rt = Rt { }; }\n    fn rewire() { let d = Doubler { }; self.rt = Rt { performer: d }; }\n    run() { self.rewire(); }\n}\nfn main() { Work { }; }\n";
const PARAM_WITH_A_BAD_CALLER: &str = "interface Performer { fn perform(x: Int) -> Int; }\nlocus Doubler { fn perform(x: Int) -> Int { return x * 2; } }\nlocus Attempt { params { performer: Performer; } run() { let x = self.performer.perform(1); } }\nlocus Rt { params { performer: Performer = Doubler { }; } fn go() -> Int { return self.performer.perform(2); } }\nlocus Work {\n    params { rt: Rt = Rt { }; }\n    fn rewire(p: Performer) { self.rt = Rt { performer: p }; }\n    run() { let d = Doubler { }; self.rewire(d); }\n}\nfn main() { Work { }; }\n";
const PARAM_WITH_A_GOOD_CALLER: &str = "interface Performer { fn perform(x: Int) -> Int; }\nlocus Doubler { fn perform(x: Int) -> Int { return x * 2; } }\nlocus Attempt { params { performer: Performer; } run() { let x = self.performer.perform(1); } }\nlocus Rt { params { performer: Performer = Doubler { }; } fn go() -> Int { return self.performer.perform(2); } }\nlocus Work {\n    params { rt: Rt = Rt { }; performer: Performer = Doubler { }; }\n    fn rewire(p: Performer) { self.rt = Rt { performer: p }; }\n    run() { self.rewire(self.performer); }\n}\nfn main() { Work { }; }\n";
const HANDLER_LOCAL_INTO_RESIDENT: &str = "interface Performer { fn perform(x: Int) -> Int; }\nlocus Doubler { fn perform(x: Int) -> Int { return x * 2; } }\nlocus Attempt { params { performer: Performer; } run() { let x = self.performer.perform(1); } }\nlocus Rt { params { performer: Performer = Doubler { }; } fn go() -> Int { return self.performer.perform(2); } }\ntype Ping { n: Int = 0; }\ntopic Pings { payload: Ping; subject: \"t.pings\"; }\nlocus Keeper { params { d: Doubler = Doubler { }; } fn go() -> Int { return self.d.perform(3); } }\nlocus Work {\n    accept(k: Keeper) { }\n    bus { subscribe Pings as on_ping; }\n    fn on_ping(p: Ping) { let d = Doubler { }; Keeper { d: d }; }\n    run() { }\n}\nfn main() { Work { }; }\n";
const LOCAL_INTO_FRAME_BINDING: &str = "interface Performer { fn perform(x: Int) -> Int; }\nlocus Doubler { fn perform(x: Int) -> Int { return x * 2; } }\nlocus Attempt { params { performer: Performer; } run() { let x = self.performer.perform(1); } }\nlocus Rt { params { performer: Performer = Doubler { }; } fn go() -> Int { return self.performer.perform(2); } }\nfn main() {\n    let d = Doubler { };\n    let r = Rt { performer: d };\n    println(r.go());\n}\n";
const BIRTH_SCOPED: &str = "interface Performer { fn perform(x: Int) -> Int; }\nlocus Doubler { fn perform(x: Int) -> Int { return x * 2; } }\nlocus Attempt { params { performer: Performer; } run() { let x = self.performer.perform(1); } }\nlocus Rt { params { performer: Performer = Doubler { }; } fn go() -> Int { return self.performer.perform(2); } }\nlocus Rows { params { n: Int = 0; } fn count() -> Int { return self.n; } }\nlocus Step {\n    params { rows: Rows = Rows { }; mine: Int = 0; }\n    birth() { self.mine = self.rows.count(); }\n    run() { println(self.mine); }\n}\nlocus Owner {\n    accept(s: Step) { }\n    run() { let rows = Rows { n: 4 }; Step { rows: rows }; }\n}\nfn main() { Owner { }; }\n";
const KEY_IN_BIRTH: &str = "interface Performer { fn perform(x: Int) -> Int; }\nlocus Doubler { fn perform(x: Int) -> Int { return x * 2; } }\nlocus Attempt { params { performer: Performer; } run() { let x = self.performer.perform(1); } }\nlocus Rt { params { performer: Performer = Doubler { }; } fn go() -> Int { return self.performer.perform(2); } }\ntype Tick { n: Int = 0; }\ntopic Ticks { payload: Tick; subject: \"t.ticks\"; keyed_by n; }\nlocus Feed {\n    params { n: Int = 0; seen: Int = 0; }\n    bus { subscribe Ticks as on_tick where key == self.n; }\n    birth() { self.n = 7; }\n    fn on_tick(t: Tick) { self.seen = self.seen + 1; }\n    run() { }\n}\nfn main() { Feed { }; }\n";

#[test]
fn a_frame_local_into_a_child_of_self_is_refused() {
    let (ok, out) = check(LOCAL_INTO_ACCEPTED, "accepted");
    assert!(!ok && out.contains(RULE) && out.contains("`Attempt.performer`") && out.contains("`d` is a `let` of `Work.run`"), "{out}");
    let (ok, out) = check(LOCAL_INTO_SELF_FIELD, "self_field");
    assert!(!ok && out.contains(RULE) && out.contains("`Rt.performer`") && out.contains("`Work.rewire`"), "{out}");
}

#[test]
fn a_field_of_self_into_a_child_of_self_is_sound() {
    let (ok, out) = check(SELF_FIELD_INTO_ACCEPTED, "self_into_accepted");
    assert!(ok && !out.contains(RULE), "{out}");
}

#[test]
fn a_parameter_is_asked_of_every_caller() {
    let (ok, out) = check(PARAM_WITH_A_BAD_CALLER, "bad_caller");
    assert!(
        !ok && out.contains(RULE) && out.contains("`p` is a parameter of `Work.rewire`") && out.contains("At the call in `run`") && out.contains("the argument `d` is a `let` of that frame"),
        "the witness names the caller and its binding: {out}"
    );
    let (ok, out) = check(PARAM_WITH_A_GOOD_CALLER, "good_caller");
    assert!(ok && !out.contains(RULE), "a caller handing a field of its own self is sound: {out}");
}

#[test]
fn a_handlers_local_into_a_resident_is_refused() {
    let (ok, out) = check(HANDLER_LOCAL_INTO_RESIDENT, "handler");
    assert!(!ok && out.contains(RULE) && out.contains("`Keeper.d`") && out.contains("`Work.on_ping`"), "{out}");
}

#[test]
fn a_frame_local_into_a_frame_binding_is_sound() {
    let (ok, out) = check(LOCAL_INTO_FRAME_BINDING, "frame");
    assert!(ok && !out.contains(RULE), "{out}");
}

#[test]
fn a_borrow_read_only_in_birth_is_birth_scoped() {
    let (ok, out) = check(BIRTH_SCOPED, "birth");
    assert!(ok && !out.contains(RULE), "the engine's shape — rows handed in, copied in birth, never read again: {out}");
}

#[test]
fn a_key_assigned_in_birth_is_noticed() {
    let (ok, out) = check(KEY_IN_BIRTH, "key");
    assert!(ok, "a notice, not a refusal: {out}");
    assert!(out.contains("warning:") && out.contains("GH #737") && out.contains("keys its subscription to `Ticks` by `self.n`"), "{out}");
}
