//! GH #721 — `hale check <seed>` refuses a bare identifier in VALUE
//! position that nothing binds, the way `hale build` always did.
//!
//! The reported shape was a one-character typo:
//!
//! ```hale
//! fn main() { let total = 1; println("" + totl); }
//! ```
//!
//! `hale check .` answered `ok: 1 file(s) typechecked` and `hale
//! verify .` agreed, because an unresolved identifier typed as
//! `Unknown` and `Unknown` is permissive everywhere. The program then
//! failed to build with `Unsupported("unknown identifier `totl`")` and
//! no source location. An organization whose admission gate is
//! check + verify let the candidate through (downstream handoff).
//!
//! F.18 (GH #583) fixed the same hole for CALLEES. This is the
//! non-call half, and it follows the same whole-vs-partial line: one
//! file of a multi-file seed legitimately reads a const a sibling
//! file declares, so checked alone it keeps the permissive reading.

use std::path::{Path, PathBuf};
use std::process::Command;

const MSG: &str = "unknown identifier `totl`";
const WHY: &str =
    "no binding, param, const or declaration with that name is in scope";

const TYPO: &str = "fn main() {\n    let total = 1;\n    println(\"\" + totl);\n}\n";

fn seed(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let d: PathBuf = std::env::temp_dir()
        .join(format!("hale_unknown_ident_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    for (name, src) in files {
        let p = d.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, src).unwrap();
    }
    d
}

fn hale(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(Path::new("/"))
        .output()
        .expect("hale");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// One file, one seed: check refuses it, at the typo's own span, and
/// names the spelling that works.
#[test]
fn the_minimal_typo_fails_check_at_its_span() {
    let d = seed("minimal", &[("main.hl", TYPO)]);
    let (ok, out) = hale(&["check", &d.to_string_lossy()]);
    assert!(!ok, "check must fail: {out}");
    assert!(out.contains(MSG) && out.contains(WHY), "{out}");
    // `totl` is on line 3, column 18 — the identifier, not the
    // statement and not the file.
    assert!(out.contains(":3:18:"), "span must point at the typo: {out}");
    assert!(
        out.contains("did you mean `total`?"),
        "a local one edit away is the likely intent: {out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// The machine-readable stream carries it too — a gate reading
/// `--json` must not see an empty diagnostic list.
#[test]
fn json_diagnostics_carry_it() {
    let d = seed("json", &[("main.hl", TYPO)]);
    let (ok, out) = hale(&["check", &d.to_string_lossy(), "--json"]);
    assert!(!ok, "{out}");
    let line = out
        .lines()
        .find(|l| l.contains(MSG))
        .unwrap_or_else(|| panic!("no json row for the typo: {out}"));
    assert!(line.contains("\"line\":3"), "{line}");
    assert!(line.contains("\"col\":18"), "{line}");
    assert!(line.contains("\"severity\":\"error\""), "{line}");
    let _ = std::fs::remove_dir_all(&d);
}

/// `hale verify` is the discipline gate the organization runs; it
/// fails on the same finding.
#[test]
fn verify_fails_on_it() {
    let d = seed("verify", &[("main.hl", TYPO)]);
    let (ok, out) = hale(&["verify", &d.to_string_lossy()]);
    assert!(!ok && out.contains(MSG), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

/// The build path reports it the same way. `hale build` bundles
/// exactly what codegen compiles, so the program is whole there too —
/// and the pre-#721 answer was a spanless `Unsupported(...)` from the
/// backend, the failure mode the issue names last.
#[test]
fn the_build_path_reports_it_with_a_span() {
    let d = seed("build", &[("main.hl", TYPO)]);
    let f = d.join("main.hl");
    let (ok, out) = hale(&["build", &f.to_string_lossy()]);
    assert!(!ok, "{out}");
    assert!(out.contains(MSG) && out.contains(":3:18:"), "{out}");
    assert!(
        !out.contains("Unsupported"),
        "must not degrade to the backend's unlocated error: {out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// Every expression position the issue lists: bare `let` RHS,
/// annotated `let` RHS, a call argument, an `if` condition, an
/// assignment RHS, and string concatenation.
#[test]
fn every_expression_context_is_covered() {
    let src = "fn takes(n: Int) -> Int { return n; }\n\
               fn main() {\n\
               \x20   let bare = missing_a;\n\
               \x20   let ann: Int = missing_b;\n\
               \x20   let arg = takes(missing_c);\n\
               \x20   if missing_d { println(\"x\"); }\n\
               \x20   let mut m = 0;\n\
               \x20   m = missing_e;\n\
               \x20   println(\"\" + missing_f);\n\
               \x20   println(m, bare, ann, arg);\n\
               }\n";
    let d = seed("contexts", &[("main.hl", src)]);
    let (ok, out) = hale(&["check", &d.to_string_lossy()]);
    assert!(!ok, "{out}");
    for who in [
        "missing_a", "missing_b", "missing_c", "missing_d", "missing_e",
        "missing_f",
    ] {
        assert!(
            out.contains(&format!("unknown identifier `{}`", who)),
            "{} unreported: {}",
            who,
            out
        );
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// A seed with a cross-seed import is still a whole program: the
/// typo is reported, and the imported const and fn are not.
#[test]
fn an_imported_seed_reports_the_typo_and_not_the_import() {
    let d = seed(
        "imported",
        &[
            (
                "lib/lib.hl",
                "const LIB_CAP: Int = 7;\n\
                 fn lib_double(n: Int) -> Int { return n * 2; }\n",
            ),
            (
                "app/main.hl",
                "import \"../lib\" as lib;\n\
                 fn main() {\n\
                 \x20   let cap = lib::LIB_CAP;\n\
                 \x20   println(cap, \" \", lib::lib_double(2));\n\
                 }\n",
            ),
        ],
    );
    let (ok, out) = hale(&["check", &d.join("app").to_string_lossy()]);
    assert!(ok, "an imported const / fn is bound: {out}");

    std::fs::write(
        d.join("app/main.hl"),
        "import \"../lib\" as lib;\n\
         fn main() {\n\
         \x20   let cap = lib::LIB_CAP;\n\
         \x20   println(cap, \" \", capp);\n\
         }\n",
    )
    .unwrap();
    let (ok, out) = hale(&["check", &d.join("app").to_string_lossy()]);
    assert!(!ok && out.contains("unknown identifier `capp`"), "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

/// The unknowns that are NOT typos, in one program: a chain's `it`,
/// the `err` bound on an `or fail` payload, a `where key ==` filter, an
/// enum variant, a top-level const, a multi-segment stdlib path, and
/// every shape of match-arm binder (bare, tuple, constructor payload).
///
/// Each of these reaches a name through something other than a `let`,
/// and a rule that only knew about `let` would refuse all of them.
#[test]
fn legitimate_unknowns_still_check_clean() {
    let src = "type Color = enum { Red, Green };\n\
        const CAP: Int = 4;\n\
        type Box { n: Int = 0; }\n\
        type Ping { id: String = \"\"; n: Int = 0; }\n\
        topic Pings { payload: Ping; subject: \"t.pings\"; keyed_by id; }\n\
        fn pick(c: Color) -> Int {\n\
        \x20   return match c { Color::Red -> 1, Color::Green -> 2 };\n\
        }\n\
        fn sign_of(n: Int) -> String {\n\
        \x20   return match n { v if v < 0 -> \"neg\", 0 -> \"zero\", v -> \"pos\" };\n\
        }\n\
        fn pair(t: (Int, Int)) -> Int {\n\
        \x20   return match t { (0, 0) -> 0, (a, b) if a > b -> a + b, _ -> 0 };\n\
        }\n\
        fn over(b: bounded[Int; 8]) -> Int {\n\
        \x20   return b.filter(it > 2).count();\n\
        }\n\
        fn src(ok: Bool) -> Int fallible(Box) {\n\
        \x20   if !ok { fail Box { n: 1 }; }\n\
        \x20   return 1;\n\
        }\n\
        fn wrapped(ok: Bool) -> Int fallible(Box) {\n\
        \x20   return src(ok) or fail Box { n: err.n };\n\
        }\n\
        locus Sub {\n\
        \x20   params { key: String = \"\"; seen: Int = 0; }\n\
        \x20   bus { subscribe Pings as on_ping where key == self.key; }\n\
        \x20   fn on_ping(p: Ping) { self.seen = self.seen + p.n; }\n\
        }\n\
        main locus App {\n\
        \x20   params { s: Sub = Sub { key: \"a\" }; }\n\
        \x20   bus { publish Pings; }\n\
        \x20   run() {\n\
        \x20       let v = wrapped(true) or - 1;\n\
        \x20       let t = std::str::pad_left(\"a\", 3, \" \");\n\
        \x20       Pings <- Ping { id: \"a\", n: 1 };\n\
        \x20       println(v, \" \", pick(Color::Red), \" \", sign_of(0 - 1), \" \",\n\
        \x20               pair((1, 2)), \" \", t, \" \", CAP, \" \", self.s.seen);\n\
        \x20   }\n\
        }\n\
        fn main() { App { }; }\n";
    let d = seed("legit", &[("main.hl", src)]);
    let (ok, out) = hale(&["check", &d.to_string_lossy()]);
    assert!(ok, "no finding expected: {out}");
    let _ = std::fs::remove_dir_all(&d);
}

/// One file of a multi-file seed reads what a sibling declares. The
/// leniency is load-bearing — the compiler's own seeds do this — so a
/// FILE target keeps the permissive reading and only the seed is held
/// to the rule.
#[test]
fn a_single_file_of_a_seed_keeps_the_permissive_reading() {
    let d = seed(
        "partial",
        &[
            ("consts.hl", "const LIMIT: Int = 3;\n"),
            (
                "main.hl",
                "fn main() { println(LIMIT); }\n",
            ),
        ],
    );
    let (ok, out) = hale(&["check", &d.join("main.hl").to_string_lossy()]);
    assert!(ok, "one file is not held to the whole-seed rule: {out}");
    let (ok, out) = hale(&["check", &d.to_string_lossy()]);
    assert!(ok, "and the seed itself binds it: {out}");
    let _ = std::fs::remove_dir_all(&d);
}

/// A bare callee that names nothing keeps F.18's message — the one
/// that knows a callee may also be a fn pointer or a generic fn — and
/// gets it exactly once.
#[test]
fn a_bare_unknown_callee_reports_once() {
    let d = seed("callee", &[("main.hl", "fn main() { println(nothing_here()); }\n")]);
    let (ok, out) = hale(&["check", &d.to_string_lossy(), "--json"]);
    assert!(!ok, "{out}");
    let rows: Vec<&str> =
        out.lines().filter(|l| l.contains("nothing_here")).collect();
    assert_eq!(rows.len(), 1, "one mistake, one message: {out}");
    assert!(
        rows[0].contains("no free fn, generic fn or fn-pointer binding"),
        "{}",
        rows[0]
    );
    let _ = std::fs::remove_dir_all(&d);
}
