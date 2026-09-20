//! GH #833 — an imported type in ANNOTATION position is checked like
//! a local one.
//!
//! `resolve_type_expr` typed every multi-segment path that was not in
//! the stdlib `PATH_RENAMES` table as `Ty::Unknown`. The merge's
//! `apply_qualified_path_renames` hid most of that: it collapses
//! `lib::Thing` to the mangled declaration before typecheck, so a
//! signature, a struct field, a locus `params` entry, a `capacity`
//! slot and an alias target were all typed after all. But that pass
//! walks TOP-LEVEL declarations only — never a fn or method body — so
//! the one annotation position that lives in a body,
//! `let x: T` (and `let (a, b): T`), reached the resolver still
//! spelled `lib::Thing` and came back `Unknown`.
//!
//! `let t: lib::Thing = "x";` therefore passed `hale check` with
//! `ok: 1 file(s) typechecked` and died at build with an unlocated
//! `Unsupported("fn `take` arg 0 type mismatch: expected
//! TypeRef("__lib_lib_types_Thing"), got String")` — check accepting
//! what the build refuses, which is the worst failure mode the
//! toolchain has.
//!
//! The fix resolves the path in the resolver instead of widening that
//! pre-pass, so the answer no longer depends on which positions an
//! earlier walk happens to reach. The `let` cases below are the ones
//! that bite; the rest are the regression net around them — each
//! position asserted to keep naming `lib::Thing`, so a change to
//! either layer has to say so here.
//!
//! Also locked in: the right initializer types, builds and runs in
//! every position; a local alias OF an imported type (GH #759 /
//! PR #832) is transparent through the import; two aliases for one
//! library are one type; `--json` carries the located record; and a
//! single FILE of a multi-file seed — whose `import` line lives in a
//! sibling — keeps the permissive `Unknown` it has always had.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The library seed every case below imports.
const LIB: &str = r#"
type Thing { n: Int = 0; }

fn make() -> Thing {
    return Thing { n: 7 };
}
"#;

/// Each `main.hl` starts with a newline, so line 1 is blank and the
/// `import` is line 2.
const LET: &str = r#"
import "lib" as lib;
fn main() {
    let t: lib::Thing = "x";
    println("n=", to_string(t.n));
}
"#;

/// The same annotation inside a LOCUS METHOD body — the mangler's
/// walk does not reach a method body either.
const LET_IN_METHOD: &str = r#"
import "lib" as lib;
locus Holder {
    params { n: Int = 0; }
    fn peek() -> Int {
        let t: lib::Thing = "x";
        return t.n + self.n;
    }
}
fn main() {
    let h = Holder { n: 1 };
    println("n=", to_string(h.peek()));
}
"#;

/// Typing the annotation also puts field and method access behind it
/// back under the checker: `Unknown` waved every one of them through.
const FIELD_ON_ANNOTATED_LET: &str = r#"
import "lib" as lib;
fn main() {
    let t: lib::Thing = lib::make();
    println("n=", to_string(t.nope));
}
"#;

const FN_PARAM: &str = r#"
import "lib" as lib;
fn take(t: lib::Thing) -> Int {
    return t.n;
}
fn main() {
    println("n=", to_string(take("x")));
}
"#;

const FN_RETURN: &str = r#"
import "lib" as lib;
fn build() -> lib::Thing {
    return "x";
}
fn main() {
    println("n=", to_string(build().n));
}
"#;

const STRUCT_FIELD: &str = r#"
import "lib" as lib;
type Wrap { t: lib::Thing = lib::Thing { }; }
fn main() {
    let w = Wrap { t: "x" };
    println("n=", to_string(w.t.n));
}
"#;

const PARAMS_FIELD: &str = r#"
import "lib" as lib;
locus Holder {
    params { t: lib::Thing = lib::Thing { }; }
    fn peek() -> Int {
        return self.t.n;
    }
}
fn main() {
    let h = Holder { t: "x" };
    println("n=", to_string(h.peek()));
}
"#;

const CAPACITY_SLOT: &str = r#"
import "lib" as lib;
@form(vec)
locus Bag {
    capacity { heap things of lib::Thing; }
}
fn main() {
    let b = Bag { };
    b.push("x");
    println("n=", to_string(b.len()));
}
"#;

/// GH #759 / PR #832: a LOCAL alias of an imported type. The alias is
/// transparent, so the mismatch reads as `lib::Thing` — the alias
/// name is a spelling, not a type.
const ALIAS_OF_IMPORTED: &str = r#"
import "lib" as lib;
type Local = lib::Thing;
fn main() {
    let t: Local = "x";
    println("n=", to_string(t.n));
}
"#;

/// Every annotation position, filled correctly. Must check, build and
/// run — the fix must not refuse what the build accepts.
const VALID: &str = r#"
import "lib" as lib;
type Wrap { t: lib::Thing = lib::Thing { }; }
type Local = lib::Thing;
locus Holder {
    params { t: lib::Thing = lib::Thing { }; }
    fn peek() -> Int {
        return self.t.n;
    }
}
fn take(t: lib::Thing) -> Int {
    return t.n;
}
fn build() -> lib::Thing {
    return lib::make();
}
fn main() {
    let direct: lib::Thing = lib::make();
    let aliased: Local = lib::make();
    let w = Wrap { t: lib::make() };
    let h = Holder { t: lib::make() };
    println("sum=", to_string(take(direct) + aliased.n + w.t.n
        + h.peek() + build().n));
}
"#;

/// Two aliases, one library. `b::Thing` must satisfy an `a::Thing`
/// annotation: the rename rows differ, the mangled declaration they
/// name does not (PR #819's value-identity shape, at the type level).
const TWO_ALIASES: &str = r#"
import "lib" as a;
import "lib" as b;
fn via_a(t: a::Thing) -> Int {
    return t.n;
}
fn main() {
    let t: b::Thing = b::make();
    println("n=", to_string(via_a(t)));
}
"#;

/// The permissive control: `helper.hl` names `lib::Thing` but the
/// `import` line lives in its SIBLING, so checking this one file
/// resolves no import and the annotation must stay `Unknown`. One
/// file of a multi-file seed is not a whole program.
const SIBLING_HELPER: &str = r#"
fn helper() -> lib::Thing {
    return "x";
}
"#;

const SIBLING_MAIN: &str = r#"
import "lib" as lib;
fn main() {
    let t: lib::Thing = helper();
    println("n=", to_string(t.n));
}
"#;

/// An app seed whose `main.hl` imports a sibling `lib/` seed.
fn app_seed(tag: &str, main_src: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join(format!("hale_i833_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("lib")).expect("mkdir seed");
    std::fs::write(d.join("lib").join("types.hl"), LIB).expect("write lib");
    std::fs::write(d.join("main.hl"), main_src).expect("write main");
    d
}

fn run(cmd: &str, target: &Path, extra: &[&str]) -> (String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg(cmd)
        .args(extra)
        .arg(target)
        .output()
        .expect("run hale");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        out.status.code().unwrap_or(-1),
    )
}

/// One case: `check` must reject, name `lib::Thing` in the author's
/// spelling, and locate the diagnostic in `main.hl`.
fn assert_rejects(tag: &str, src: &str, expected: &str) {
    let d = app_seed(tag, src);
    let (out, code) = run("check", &d, &[]);
    assert_ne!(
        code, 0,
        "`check` must reject the {} case:\n{}",
        tag, out
    );
    assert!(
        out.contains(expected),
        "expected {:?} in the {} diagnostic:\n{}",
        expected,
        tag,
        out
    );
    assert!(
        out.contains("main.hl:"),
        "the {} diagnostic must be LOCATED in main.hl:\n{}",
        tag,
        out
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn let_annotation_is_checked() {
    assert_rejects(
        "let",
        LET,
        "let `t`: expected `lib::Thing`, got `String`",
    );
}

#[test]
fn let_annotation_in_a_locus_method_is_checked() {
    assert_rejects(
        "method",
        LET_IN_METHOD,
        "let `t`: expected `lib::Thing`, got `String`",
    );
}

#[test]
fn field_access_behind_an_annotated_let_is_checked() {
    assert_rejects(
        "fieldaccess",
        FIELD_ON_ANNOTATED_LET,
        "no field `nope` on `lib::Thing`",
    );
}

#[test]
fn fn_param_annotation_is_checked() {
    assert_rejects(
        "param",
        FN_PARAM,
        "argument 0 type mismatch: expected `lib::Thing`, got `String`",
    );
}

#[test]
fn fn_return_annotation_is_checked() {
    assert_rejects(
        "ret",
        FN_RETURN,
        "return: expected `lib::Thing`, got `String`",
    );
}

#[test]
fn struct_field_annotation_is_checked() {
    assert_rejects(
        "field",
        STRUCT_FIELD,
        "field `t` expects `lib::Thing`, got `String`",
    );
}

#[test]
fn params_field_annotation_is_checked() {
    assert_rejects(
        "params",
        PARAMS_FIELD,
        "field `t` expects `lib::Thing`, got `String`",
    );
}

#[test]
fn capacity_slot_annotation_is_checked() {
    assert_rejects(
        "capacity",
        CAPACITY_SLOT,
        "expected `lib::Thing`, got `String`",
    );
}

#[test]
fn alias_of_imported_type_is_checked() {
    // The alias is transparent (GH #759): the message names the
    // TARGET, `lib::Thing`, not the local spelling.
    assert_rejects(
        "alias",
        ALIAS_OF_IMPORTED,
        "let `t`: expected `lib::Thing`, got `String`",
    );
}

#[test]
fn json_carries_the_located_record() {
    let d = app_seed("json", LET);
    let (out, code) = run("check", &d, &["--json"]);
    assert_ne!(code, 0, "`check --json` must reject:\n{}", out);
    let line = out
        .lines()
        .find(|l| l.starts_with('{'))
        .unwrap_or_else(|| panic!("no NDJSON record:\n{}", out));
    for needle in [
        "\"line\":4",
        "\"severity\":\"error\"",
        "\"kind\":\"type error\"",
        "expected `lib::Thing`, got `String`",
        "main.hl",
    ] {
        assert!(
            line.contains(needle),
            "expected {:?} in the record:\n{}",
            needle,
            line
        );
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn every_annotation_position_filled_correctly_checks_and_runs() {
    let d = app_seed("valid", VALID);
    let (out, code) = run("check", &d, &[]);
    assert_eq!(code, 0, "`check` must accept the valid seed:\n{}", out);
    let (out, code) = run("run", &d, &[]);
    assert_eq!(code, 0, "`run` must accept the valid seed:\n{}", out);
    // 7 (take) + 7 (alias) + 7 (struct field) + 7 (params) + 7 (ret)
    assert!(
        out.contains("sum=35"),
        "expected every position to carry the imported value:\n{}",
        out
    );
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn two_aliases_for_one_library_are_one_type() {
    let d = app_seed("aliases", TWO_ALIASES);
    let (out, code) = run("check", &d, &[]);
    assert_eq!(
        code, 0,
        "`b::Thing` must satisfy an `a::Thing` annotation:\n{}",
        out
    );
    let (out, code) = run("run", &d, &[]);
    assert_eq!(code, 0, "the two-alias seed must run:\n{}", out);
    assert!(out.contains("n=7"), "unexpected output:\n{}", out);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_single_file_of_a_multi_file_seed_stays_permissive() {
    let d = app_seed("sibling", SIBLING_MAIN);
    std::fs::write(d.join("helper.hl"), SIBLING_HELPER)
        .expect("write helper");

    // `helper.hl` alone: nothing here resolves `lib`, so the
    // annotation is a path this bundle cannot see. Permissive, as it
    // has always been — the rule is for WHOLE programs.
    let (out, code) = run("check", &d.join("helper.hl"), &[]);
    assert_eq!(
        code, 0,
        "one file of a multi-file seed must stay permissive:\n{}",
        out
    );

    // The whole seed: every import resolved, so the same annotation
    // is checked and the mismatch is located in `helper.hl`.
    let (out, code) = run("check", &d, &[]);
    assert_ne!(code, 0, "the whole seed must be checked:\n{}", out);
    assert!(
        out.contains("return: expected `lib::Thing`, got `String`"),
        "expected the located mismatch:\n{}",
        out
    );
    assert!(
        out.contains("helper.hl:"),
        "expected the diagnostic against helper.hl:\n{}",
        out
    );
    let _ = std::fs::remove_dir_all(&d);
}
