//! F.40 phase 3, C4 (the first judgment migration): the unowned-subscriber
//! rule, `spec/semantics.md` § Placement block, type-check rule 20, as
//! `hale check` says it.
//!
//! The rule used to keep a locus index of its own, keyed by the bare name
//! (the last declaration of a name won), and matched an `accept` to a
//! birth by the accept type's last path segment. It reads the ownership
//! graph now, whose `accept` edges are keyed by declaration identity, and
//! each test here pins one class of what moved:
//!
//! 1. An `accept` that names the subscriber through an alias owns it, so
//!    the birth stops erroring (the name match missed the alias).
//! 2. An `accept` whose type merely shares the subscriber's last segment
//!    does not own it, so the birth starts erroring: a path that names
//!    nothing, and an import path naming another seed's declaration.
//! 3. An `accept` of one specialization of a generic template does not
//!    own a birth of another, so that birth starts erroring; a birth the
//!    binding declares as the accepted specialization stays owned.
//! 4. Two loci of one name: the graph judges the first in declaration
//!    order, as the scope resolves the name, and the diagnostic says
//!    which (the index kept the last, so a subscriber declared first went
//!    unjudged).
//! 5. Where the graph cannot decide, nothing is reported: an open world
//!    (no entry, so a consumer may complete the tower), a generic
//!    template no declared type specializes.
//! 6. The nearest accepting ancestor owns a birth in a handler as it owns
//!    any other (the bubble lowering performs), so the birth stops
//!    erroring.
//! 7. ... but only when one accepts it on EVERY construction path of the
//!    handler's locus, which the placement table records: a locus built
//!    under an accepting parent and also directly in `fn main` is
//!    refused, and the diagnostic names the `main` path; a path the table
//!    records as a hole proves no owner either.
//!
//! The programs are escaped string literals rather than raw strings:
//! `hale-corpus` harvests raw-string literals out of test files, and these
//! are deliberately diagnostic.

use std::path::{Path, PathBuf};
use std::process::Command;

const MESSAGES: &str = "type Trigger { n: Int; }\ntype Data { v: Int; }\n";

/// `Child`, a subscriber, and `Disp`, whose `on_trig` handler births
/// `birth`; `Disp` declares `accept`.
fn handler_birth(child: &str, accept: &str, birth: &str) -> String {
    format!(
        "{MESSAGES}{child}
locus Disp {{
    {accept}
    bus {{ subscribe \"trig\" as on_trig of type Trigger; }}
    fn on_trig(t: Trigger) {{ {birth} }}
}}
"
    )
}

const CHILD: &str = "\
locus Child {
    params { id: Int = 0; }
    bus { subscribe \"data\" as on_data of type Data; }
    fn on_data(d: Data) { println(\"got\"); }
}
";

const CELL: &str = "\
locus Cell<T> {
    params { id: Int = 0; }
    bus { subscribe \"data\" as on_data of type Data; }
    fn on_data(d: Data) { println(\"got\"); }
}
";

const MAIN: &str = "fn main() { Disp { }; }\n";

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hale_check_unowned_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn seed(root: &Path, name: &str, text: &str) -> PathBuf {
    let d = root.join(name);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("main.hl"), text).unwrap();
    d
}

fn check(seed: &Path) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("check")
        .arg(seed)
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    (
        out.status.success(),
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    )
}

/// The diagnostic's first sentence, for `child` born in `Disp.on_trig`.
fn unowned(child: &str) -> String {
    format!(
        "locus `{child}` declares `bus subscribe` but is instantiated unowned inside \
         `Disp`'s bus handler `on_trig`. A bus handler returns after each message, so the \
         locals it binds dissolve immediately — `{child}`'s subscription would never fire \
         for a later message."
    )
}

fn assert_owned(tag: &str, program: &str, why: &str) {
    let root = scratch(tag);
    let (ok, out) = check(&seed(&root, "app", program));
    assert!(ok && !out.contains("instantiated unowned"), "{why}:\n{out}");
    let _ = std::fs::remove_dir_all(&root);
}

fn assert_unowned(tag: &str, program: &str, child: &str, why: &str) -> String {
    let root = scratch(tag);
    let (ok, out) = check(&seed(&root, "app", program));
    assert!(!ok, "{why}:\n{out}");
    assert_eq!(out.matches("instantiated unowned").count(), 1, "{why}: one finding:\n{out}");
    assert!(out.contains(&unowned(child)), "{why}: the rule's wording, naming `{child}`:\n{out}");
    let _ = std::fs::remove_dir_all(&root);
    out
}

/// Class 1: `type Kid = Child; accept(c: Kid)` owns a `Child` born in
/// the handler. The name match compared `Kid` with `Child` and refused.
#[test]
fn an_accept_naming_the_subscriber_through_an_alias_owns_it() {
    let program = format!(
        "{}type Kid = Child;\n{MAIN}",
        handler_birth(CHILD, "accept(c: Kid) { }", "Child { id: 1 };")
    );
    assert_owned("alias", &program, "an aliased accept names the subscriber's declaration");
}

/// Class 2: `accept(c: other::Child)` names nothing this program
/// declares; it shares only `Child`'s last segment, which the name match
/// took for ownership.
#[test]
fn an_accept_sharing_only_the_last_segment_does_not_own_it() {
    let program =
        format!("{}{MAIN}", handler_birth(CHILD, "accept(c: other::Child) { }", "Child { id: 1 };"));
    assert_unowned("last_segment", &program, "Child", "a path that names nothing owns nothing");
}

/// Class 2, across seeds: `accept(c: lib::Child)` names the library's
/// `Child`, a declaration of its own; the consumer's `Child` born in the
/// handler is another locus, which the library's accept does not own.
#[test]
fn an_import_path_owns_its_own_declaration_not_a_namesake() {
    let root = scratch("import_path");
    seed(&root, "lib", &format!("{MESSAGES}{CHILD}"));
    let program = format!(
        "import \"../lib\" as lib;\n{}{MAIN}",
        handler_birth(CHILD, "accept(c: lib::Child) { }", "Child { id: 1 };")
    );
    let (ok, out) = check(&seed(&root, "app", &program));
    assert!(!ok, "the library's Child is not the consumer's:\n{out}");
    assert!(out.contains(&unowned("Child")), "the rule's wording:\n{out}");
    let _ = std::fs::remove_dir_all(&root);
}

/// Class 3: `accept(c: Cell<Int>)` owns the `Cell<Int>` specialization; a
/// birth declared `Cell<String>` shares only the template.
#[test]
fn an_accept_of_another_specialization_does_not_own_it() {
    let program = format!(
        "{}{MAIN}",
        handler_birth(CELL, "accept(c: Cell<Int>) { }", "let x: Cell<String> = Cell { id: 1 };")
    );
    assert_unowned("template", &program, "Cell", "a template match is not ownership");
}

/// Class 3's other half: the birth its binding declares as the accepted
/// specialization is owned, as lowering accepts it.
#[test]
fn an_accept_of_the_births_specialization_owns_it() {
    let program = format!(
        "{}{MAIN}",
        handler_birth(CELL, "accept(c: Cell<Int>) { }", "let x: Cell<Int> = Cell { id: 1 };")
    );
    assert_owned("specialization", &program, "the accepted specialization is owned");
}

/// Class 4: two loci named `Child` (a duplicate the checker refuses on
/// its own). The graph judges the first, the subscriber, as the scope
/// resolves the name, and the diagnostic points at it; the index kept
/// the last, which subscribes to nothing, and said nothing.
#[test]
fn two_loci_of_one_name_judge_the_first_declaration() {
    let program = format!(
        "{}module b {{\n    locus Child {{ params {{ id: Int = 0; }} }}\n}}\n{MAIN}",
        handler_birth(CHILD, "", "Child { id: 1 };")
    );
    let out = assert_unowned("duplicate", &program, "Child", "the first declaration is judged");
    assert!(out.contains("duplicate top-level name `Child`"), "the duplicate is its own error:\n{out}");
    assert!(
        out.contains(
            "note: the declaration judged: the first, in declaration order, of the 2 loci named \
             `Child` at "
        ),
        "the diagnostic names the declaration it judged:\n{out}"
    );
    assert!(out.contains("app/main.hl:3:1"), "the note points at the first `Child`:\n{out}");
}

/// Class 5: a seed with no entry is an open world. A consumer may accept
/// `Child` above `Disp`, so the graph cannot decide, and the rule does
/// not fire; the name match reported it.
#[test]
fn an_open_world_is_not_judged() {
    let program = handler_birth(CHILD, "", "Child { id: 1 };");
    assert_owned("open_world", &program, "unknown ownership is not proven absence");
}

/// Class 5: a generic template born with no declared type is a
/// specialization the graph cannot name, so the rule does not fire. The
/// literal is refused on its own (it takes its type arguments from the
/// site), and that is the only finding.
#[test]
fn an_unspecialized_template_is_not_judged() {
    let program = format!("{}{MAIN}", handler_birth(CELL, "", "Cell { id: 1 };"));
    let root = scratch("unspecialized");
    let (ok, out) = check(&seed(&root, "app", &program));
    assert!(!ok && out.contains("`Cell` is a generic locus"), "the literal is its own error:\n{out}");
    assert!(!out.contains("instantiated unowned"), "the graph cannot say which locus is born:\n{out}");
    let _ = std::fs::remove_dir_all(&root);
}

/// Class 6: `Disp` does not accept `Child`, but its parent `App` does,
/// and the nearest accepting ancestor owns a birth in a handler as it
/// owns any other.
#[test]
fn the_nearest_accepting_ancestor_owns_it() {
    let program = format!(
        "{}main locus App {{\n    params {{ d: Disp = Disp {{ }}; }}\n    accept(c: Child) {{ }}\n    run() {{ }}\n}}\n",
        handler_birth(CHILD, "", "Child { id: 1 };")
    );
    assert_owned("ancestor", &program, "an accepting ancestor owns the birth");
}

/// The control the classes move against: a subscriber no ancestor
/// accepts is still refused, with its wording, and the escape hatch
/// still allows it.
#[test]
fn a_subscriber_no_ancestor_accepts_is_refused() {
    let program = format!("{}{MAIN}", handler_birth(CHILD, "", "let c = Child { id: 1 };"));
    assert_unowned("control", &program, "Child", "no accept anywhere");
    let root = scratch("control_allowed");
    let dir = seed(&root, "app", &program);
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["check", "--allow-unowned-subscriber"])
        .arg(&dir)
        .output()
        .expect("hale");
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success() && !text.contains("instantiated unowned"), "{text}");
    let _ = std::fs::remove_dir_all(&root);
}

/// `Owner` accepts `Child` and builds a `Disp` as its params field;
/// `main` builds `extra` beside it.
fn owner_and_main(extra: &str) -> String {
    format!(
        "{}locus Owner {{
    params {{ d: Disp = Disp {{ }}; }}
    accept(c: Child) {{ }}
    run() {{ }}
}}
fn main() {{
    let owner = Owner {{ }};
{extra}}}
",
        handler_birth(CHILD, "", "Child { id: 1 };")
    )
}

/// Class 7, the review's program: `Disp` is built under `Owner`, which
/// accepts `Child`, and also directly in `fn main`, where nothing does.
/// The ancestor owns only the first path, so the birth is refused by
/// `hale check` and by `hale build`, and the note names the `main` path.
/// The walk the graph used to climb skipped free fns, `fn main`
/// included, and found `Owner` on every path it saw.
#[test]
fn an_ancestor_must_accept_on_every_construction_path() {
    let program = owner_and_main("    let direct = Disp { };\n");
    let out = assert_unowned("every_path", &program, "Child", "the `main` path has no acceptor");
    assert!(
        out.contains("`Disp` is built here, directly in `fn main`, and no ancestor of it accepts `Child`"),
        "the note names the path with no acceptor:\n{out}"
    );
    assert!(out.contains("app/main.hl:21:18"), "the note points at `main`'s literal:\n{out}");
    let root = scratch("every_path_build");
    let dir = seed(&root, "app", &program);
    let built = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(&dir)
        .arg("-o")
        .arg(root.join("app.bin"))
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    let text = format!("{}{}", String::from_utf8_lossy(&built.stdout), String::from_utf8_lossy(&built.stderr));
    assert!(!built.status.success() && text.contains(&unowned("Child")), "the build refuses it too:\n{text}");
    let _ = std::fs::remove_dir_all(&root);
}

/// Class 7's control: with the `main` construction removed, `Owner` is
/// on every path and owns the birth.
#[test]
fn an_ancestor_on_every_construction_path_owns_it() {
    assert_owned("every_path_owned", &owner_and_main(""), "`Owner` accepts on the only path");
}

/// Class 7: a construction path the placement table records as a hole
/// proves no owner. `Disp` is built in `Mid.run()`, `Mid` as the params
/// field of a `Shell` that `Owner` builds in its own `run()`. The table
/// does not enumerate the params subtree of a locus built only
/// dynamically, so it places no instance of `Mid`, and its literal of
/// `Disp` is a dynamic site of unknown domain: a hole. `Owner` is the
/// only acceptor the walk's edges reach, but the hole cannot be proven
/// to lie under it.
#[test]
fn a_hole_in_the_placement_table_proves_no_owner() {
    let program = format!(
        "{}locus Mid {{
    run() {{ Disp {{ }}; }}
}}
locus Shell {{
    params {{ m: Mid = Mid {{ }}; }}
}}
locus Owner {{
    accept(c: Child) {{ }}
    run() {{ Shell {{ }}; }}
}}
fn main() {{ Owner {{ }}; }}
",
        handler_birth(CHILD, "", "Child { id: 1 };")
    );
    let out = assert_unowned("hole", &program, "Child", "a hole is no proof of an owner");
    assert!(
        out.contains(
            "`Disp` is built here, in `Mid`, where the placement table knows no domain: the enclosing \
             locus has no instance the table places: the placement table records a hole, which proves \
             no ancestor that accepts `Child`"
        ),
        "the note names the hole:\n{out}"
    );
}
