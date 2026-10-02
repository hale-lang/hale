//! F.40 phase 3, C2 — a library's name is its own.
//!
//! Imported symbols are mangled under a name the load gives each library
//! (`AliasScopes::name_library`). Outside a workspace the name was the
//! library's file name, so two single-file libraries called `util.hl` in
//! different directories shared one symbol namespace: the second one's
//! declarations were merged under the first's mangled names. A
//! first-claim allocator then told them apart by import order, which
//! made a library's symbols depend on the rest of the build. The name is
//! now a function of the library's own path alone — relative to the
//! workspace root, or to the entry seed's directory for a library
//! outside the workspace — encoded injectively, so no two libraries share
//! one and no build gives one library two. A declaration's full name
//! encodes the library, the file stem and the declaration as one
//! injective tuple, so no two declarations share one either.
//!
//! The symbols are read where the load records them: the rename table,
//! `alias::Name` -> the mangled name, as `collect_checkable` returns it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn tree(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let d: PathBuf = std::env::temp_dir().join(format!("hale_library_names_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    for (name, src) in files {
        let p = d.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).expect("mkdir");
        std::fs::write(&p, src).expect("write");
    }
    d
}

fn hale(cwd: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("hale");
    (
        out.status.success(),
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    )
}

/// `alias::Name` -> the symbol it is mangled under, for the load of
/// `target`.
fn symbols(target: &Path) -> BTreeMap<String, String> {
    let loaded = hale_frontend::frontend::collect_checkable(target, &hale_frontend::source::Disk);
    let Ok((_, _, _, renames, _, _)) = loaded else {
        panic!("{} must load", target.display());
    };
    renames.into_iter().map(|(segs, mangled)| (segs.join("::"), mangled)).collect()
}

/// A library declaring `Tag` and `who()`, answering `who`.
fn lib(who: &str) -> String {
    format!("type Tag {{\n    text: String;\n}}\n\nfn who() -> String {{\n    return \"{who}\";\n}}\n")
}

/// An app importing `(path, alias)` in order, printing each alias's
/// `who()` through its own `Tag`, joined by `/`.
fn app(imports: &[(&str, &str)]) -> String {
    let mut s = String::new();
    for (path, alias) in imports {
        s.push_str(&format!("import \"{path}\" as {alias};\n"));
    }
    s.push_str("\nfn main() {\n    let mut out = \"\";\n");
    for (i, (_, alias)) in imports.iter().enumerate() {
        let sep = if i == 0 { "" } else { "/" };
        s.push_str(&format!("    let t{i} = {alias}::Tag {{ text: {alias}::who() }};\n"));
        s.push_str(&format!("    out = out + \"{sep}\" + t{i}.text;\n"));
    }
    s.push_str("    println(out);\n}\n");
    s
}

fn check_and_run(dir: &Path, want: &str) {
    let (ok, out) = hale(dir, &["check", "."]);
    assert!(ok, "check must pass in {}:\n{out}", dir.display());
    let (ok, out) = hale(dir, &["run", "."]);
    assert!(ok, "run must pass in {}:\n{out}", dir.display());
    assert_eq!(out.trim(), want, "each alias reaches its own library");
}

/// Two single-file libraries with one file name, declaring the same
/// names, and an app that imports both: each alias answers with its own
/// library's value, and a type of one is not the other's.
#[test]
fn two_single_file_libraries_sharing_a_name_stay_apart() {
    let d = tree(
        "same_file_name",
        &[
            ("one/util.hl", &lib("one")),
            ("two/util.hl", &lib("two")),
            ("app/main.hl", &app(&[("../one/util", "a"), ("../two/util", "b")])),
        ],
    );
    check_and_run(&d.join("app"), "one/two");
    let _ = std::fs::remove_dir_all(&d);
}

/// The name the first-claim allocator would have given `two/util.hl` —
/// its stem and a digest of its canonical path — for the fixture that
/// held that name already.
fn first_claim_fallback(lib: &Path) -> String {
    let mut digest: u64 = 0xcbf29ce484222325;
    for b in lib.canonicalize().unwrap().to_string_lossy().bytes() {
        digest = (digest ^ u64::from(b)).wrapping_mul(0x100000001b3);
    }
    format!("util_{:08x}", digest as u32)
}

/// Review finding 1: a third library, a DIRECTORY named exactly what
/// the old fallback generated for `two/util.hl`, beside two single-file
/// `util.hl` libraries. The allocator inserted its generated name
/// unchecked, so `c`'s declarations collided with `b`'s
/// (`duplicate top-level name c::Tag`). Each library's name is now its
/// own path, so all three check, run, and answer for themselves.
#[test]
fn a_directory_named_like_a_generated_fallback_stays_apart() {
    let d = tree("fallback_named_dir", &[("one/util.hl", &lib("one")), ("two/util.hl", &lib("two"))]);
    let third = first_claim_fallback(&d.join("two/util.hl"));
    let files = [
        (format!("three/{third}/lib.hl"), lib("three")),
        (
            "app/main.hl".to_string(),
            app(&[("../one/util", "a"), ("../two/util", "b"), (&format!("../three/{third}"), "c")]),
        ),
    ];
    for (name, src) in &files {
        let p = d.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, src).unwrap();
    }
    let app = d.join("app");
    check_and_run(&app, "one/two/three");
    let s = symbols(&app);
    assert_eq!(s["a::who"], "__lib_x2e_x2e__one__util__util__who");
    assert_eq!(s["b::who"], "__lib_x2e_x2e__two__util__util__who");
    assert_eq!(s["c::who"], format!("__lib_x2e_x2e__three__{third}___lib__who"));
    let _ = std::fs::remove_dir_all(&d);
}

/// Review finding 2: import order decided which library kept the plain
/// name. The same two libraries imported in either order now have the
/// same symbols.
#[test]
fn import_order_does_not_change_a_librarys_symbols() {
    let d = tree(
        "import_order",
        &[
            ("one/util.hl", &lib("one")),
            ("two/util.hl", &lib("two")),
            ("ab/main.hl", &app(&[("../one/util", "a"), ("../two/util", "b")])),
            ("ba/main.hl", &app(&[("../two/util", "b"), ("../one/util", "a")])),
        ],
    );
    check_and_run(&d.join("ab"), "one/two");
    check_and_run(&d.join("ba"), "two/one");
    let (ab, ba) = (symbols(&d.join("ab")), symbols(&d.join("ba")));
    assert_eq!(ab, ba, "the same libraries, the same symbols, whatever the order");
    assert_ne!(ab["a::who"], ab["b::who"]);
    let _ = std::fs::remove_dir_all(&d);
}

/// Review finding 2: two applications of one workspace, at different
/// depths and with different dependency sets, importing one shared
/// library: its symbols are the same in both, so a DTO crossing between
/// them is symbol-identical.
#[test]
fn two_applications_with_different_dependencies_share_a_librarys_symbols() {
    let d = tree(
        "two_apps",
        &[
            ("hale.toml", "[project]\nname = \"ws\"\n"),
            ("shared/messages/messages.hl", &lib("shared")),
            ("one/util.hl", &lib("one")),
            ("apps/x/main.hl", &app(&[("../../shared/messages", "m")])),
            ("deep/er/y/main.hl", &app(&[("../../../one/util", "u"), ("../../../shared/messages", "msgs")])),
        ],
    );
    check_and_run(&d.join("apps/x"), "shared");
    check_and_run(&d.join("deep/er/y"), "one/shared");
    let (x, y) = (symbols(&d.join("apps/x")), symbols(&d.join("deep/er/y")));
    assert_eq!(x["m::who"], "__lib_shared__messages___messages__who");
    assert_eq!(x["m::who"], y["msgs::who"]);
    assert_eq!(x["m::Tag"], y["msgs::Tag"]);
    let _ = std::fs::remove_dir_all(&d);
}

/// Two workspace libraries whose paths differ only by a hyphen against
/// an underscore collapsed to one name (`lib_a`) under the old
/// sanitizer. The encoding keeps them apart.
#[test]
fn workspace_libraries_that_sanitized_alike_stay_apart() {
    let d = tree(
        "hyphen_underscore",
        &[
            ("hale.toml", "[project]\nname = \"ws\"\n"),
            ("lib-a/lib.hl", &lib("hyphen")),
            ("lib_a/lib.hl", &lib("underscore")),
            ("app/main.hl", &app(&[("../lib-a", "h"), ("../lib_a", "u")])),
        ],
    );
    let app = d.join("app");
    check_and_run(&app, "hyphen/underscore");
    let s = symbols(&app);
    assert_eq!(s["h::who"], "__lib_lib_x2da___lib__who");
    assert_eq!(s["u::who"], "__lib_lib_a___lib__who");
    let _ = std::fs::remove_dir_all(&d);
}

/// The same tree at two absolute directories has the same symbols: a
/// workspace library is named relative to the workspace root, and a
/// library outside the workspace relative to the entry seed's
/// directory, so a tree moved or cloned as a whole keeps every name.
#[test]
fn a_copied_tree_keeps_its_symbols() {
    let files = [
        ("ext/util.hl", lib("ext")),
        ("ws/hale.toml", "[project]\nname = \"ws\"\n".to_string()),
        ("ws/shared/messages/messages.hl", lib("shared")),
        ("ws/app/main.hl", app(&[("../shared/messages", "m"), ("../../ext/util", "e")])),
    ];
    let files: Vec<(&str, &str)> = files.iter().map(|(n, s)| (*n, s.as_str())).collect();
    let here = tree("copy_here", &files);
    let there = tree("copy_there/nested/deeper", &files);
    let (a, b) = (symbols(&here.join("ws/app")), symbols(&there.join("ws/app")));
    assert_eq!(a, b, "a copied tree keeps its symbols");
    assert_eq!(a["e::who"], "__lib_x2e_x2e__x2e_x2e__ext__util__util__who");
    check_and_run(&there.join("ws/app"), "shared/ext");
    let _ = std::fs::remove_dir_all(&here);
    let _ = std::fs::remove_dir_all(std::env::temp_dir().join(format!(
        "hale_library_names_{}_copy_there",
        std::process::id()
    )));
}

/// Two aliases for one canonical library, spelled two ways: one copy,
/// one identity.
#[test]
fn two_aliases_for_one_library_share_its_identity() {
    let d = tree(
        "two_aliases",
        &[
            ("one/util.hl", &lib("one")),
            ("app/main.hl", &app(&[("../one/util", "a"), ("../one/../one/util", "b")])),
        ],
    );
    let app = d.join("app");
    check_and_run(&app, "one/one");
    let s = symbols(&app);
    assert_eq!(s["a::who"], s["b::who"]);
    assert_eq!(s["a::Tag"], s["b::Tag"]);
    let _ = std::fs::remove_dir_all(&d);
}

/// A library the app imports directly and that another library imports
/// too keeps one name: both aliases resolve to the one copy.
#[test]
fn a_library_reached_directly_and_through_another_keeps_one_name() {
    let d = tree(
        "direct_and_through",
        &[
            ("one/util.hl", &lib("one")),
            (
                "mid/mid.hl",
                "import \"../one/util\" as u;\n\nfn via() -> String {\n    return u::who();\n}\n",
            ),
            (
                "app/main.hl",
                "import \"../one/util\" as a;\nimport \"../mid\" as m;\n\n\
                 fn main() {\n    println(a::who() + \"/\" + m::via());\n}\n",
            ),
        ],
    );
    let app = d.join("app");
    check_and_run(&app, "one/one");
    let s = symbols(&app);
    assert_eq!(s["a::who"], "__lib_x2e_x2e__one__util__util__who");
    assert_eq!(s["u::who"], s["a::who"], "the library's one name, through either importer");
    let _ = std::fs::remove_dir_all(&d);
}

/// The re-review's pair: library `a` holds `b__util.hl`, and library
/// `a/b` holds `util.hl` (a directory import takes only the files
/// directly in it). The library names differ, `a_` and `a__b_`, but
/// joined to the raw stem by `_` both declarations were
/// `__lib_a__b__util_Tag`, and the check refused the second as a
/// duplicate. The stem is encoded and joined by `__`, so each alias
/// reaches its own `who` through its own `Tag`.
#[test]
fn a_stem_holding_a_separator_is_not_another_librarys_path() {
    let d = tree(
        "stem_separator",
        &[
            ("hale.toml", "[project]\nname = \"ws\"\n"),
            ("a/b__util.hl", &lib("1")),
            ("a/b/util.hl", &lib("2")),
            ("app/main.hl", &app(&[("../a", "a"), ("../a/b", "b")])),
        ],
    );
    let app = d.join("app");
    check_and_run(&app, "1/2");
    let s = symbols(&app);
    assert_eq!(s["a::Tag"], "__lib_a___b_x5f_util__Tag");
    assert_eq!(s["b::Tag"], "__lib_a__b___util__Tag");
    assert_eq!(s["a::who"], "__lib_a___b_x5f_util__who");
    assert_eq!(s["b::who"], "__lib_a__b___util__who");
    let _ = std::fs::remove_dir_all(&d);
}

/// Declaration names that hold separator-like text, in two files of one
/// library whose stems could have absorbed it: `q.hl` declares `s__who`
/// and `q_s.hl` declares `_who`. Joined raw by `_` both were
/// `__lib_p__q_s__who`; encoded, each keeps its own name.
#[test]
fn a_declaration_holding_a_separator_keeps_its_own_name() {
    let d = tree(
        "decl_separator",
        &[
            ("hale.toml", "[project]\nname = \"ws\"\n"),
            ("p/q.hl", "fn s__who() -> String {\n    return \"double\";\n}\n"),
            ("p/q_s.hl", "fn _who() -> String {\n    return \"leading\";\n}\n"),
            ("app/main.hl", "import \"../p\" as p;\n\nfn main() {\n    println(p::s__who() + \"/\" + p::_who());\n}\n"),
        ],
    );
    let app = d.join("app");
    check_and_run(&app, "double/leading");
    let s = symbols(&app);
    assert_eq!(s["p::s__who"], "__lib_p___q__s_x5f_who");
    assert_eq!(s["p::_who"], "__lib_p___q_s__x5fwho");
    let _ = std::fs::remove_dir_all(&d);
}

/// The compatibility boundary: a single-file library beside the entry
/// whose file name is letters and digits keeps every symbol it had —
/// `__lib_<stem>_<stem>_<name>` — for a declaration whose name neither
/// starts with `_` nor holds `__`.
#[test]
fn a_library_beside_the_entry_keeps_its_name() {
    let d = tree(
        "beside",
        &[("app/util.hl", &lib("util")), ("app/main.hl", &app(&[("util", "u")]))],
    );
    let s = symbols(&d.join("app/main.hl"));
    assert_eq!(s["u::who"], "__lib_util_util_who");
    let (ok, out) = hale(&d.join("app"), &["run", "main.hl"]);
    assert!(ok, "run must pass:\n{out}");
    assert_eq!(out.trim(), "util");
    // Named from inside the entry's directory, the entry's parent is
    // the empty path: the name is the same. (The import trace says
    // which name the load gave; an optimized binary may have inlined
    // `who` away.)
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .args(["check", "main.hl"])
        .current_dir(d.join("app"))
        .env("HALE_IMPORT_DEBUG", "1")
        .output()
        .expect("hale");
    let trace = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "check must pass:\n{trace}");
    assert!(
        trace.lines().any(|l| l.contains("util.hl is named util")),
        "the library beside the entry is named `util`:\n{trace}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// A library declaring the enum `name` with variants `Red` and `Green`.
fn enum_lib(name: &str) -> String {
    format!("type {name} = enum {{ Red, Green }};\n")
}

/// The re-review's program: an app importing `path` as `lib` and
/// matching a `lib::<name>` over `variants`, printing the label of `Red`.
fn enum_app(path: &str, name: &str, variants: &[&str]) -> String {
    let mut s = format!("import \"{path}\" as lib;\n\nfn label(c: lib::{name}) -> String {{\n    return match c {{\n");
    for v in variants {
        s.push_str(&format!("        lib::{name}::{v} -> \"{}\",\n", v.to_lowercase()));
    }
    s.push_str(&format!("    }};\n}}\n\nfn main() {{\n    println(label(lib::{name}::Red));\n}}\n"));
    s
}

/// The three ways an app reaches a library declaring the enum: a
/// directory beside the entry, a single file beside it (the
/// compatibility spelling for an ordinary name), and a single file
/// outside the entry's directory (always encoded). Each is
/// `(file, import path)`, relative to the tree, the entry at `app/`.
const ENUM_SHAPES: [(&str, &str); 3] =
    [("app/lib/defs.hl", "lib"), ("app/lib.hl", "lib"), ("one/defs.hl", "../one/defs")];

/// An ordinary name and three whose declaration is encoded in the
/// merged name, so its text is no suffix of the symbol.
const ENUM_NAMES: [&str; 4] = ["Color", "Code_x", "E__Code", "_Color"];

/// C2 re-review: a complete match over an imported enum is exhaustive
/// whatever the enum is called. The exhaustiveness reader compared the
/// merged symbol with the authored name as a suffix, so an encoded name
/// (`Code_x` is `…__Code_x5fx`) made a complete match "not exhaustive".
/// The arm's `lib::Enum` now resolves through the import table and is
/// compared with the scrutinee's declaration by identity.
#[test]
fn a_complete_match_over_an_imported_enum_is_exhaustive_whatever_its_name() {
    for (i, (file, path)) in ENUM_SHAPES.iter().enumerate() {
        for name in ENUM_NAMES {
            let d = tree(
                &format!("enum_complete_{i}_{name}"),
                &[(file, &enum_lib(name)), ("app/main.hl", &enum_app(path, name, &["Red", "Green"]))],
            );
            let (ok, out) = hale(&d.join("app"), &["check", "main.hl"]);
            assert!(ok, "a complete match over `lib::{name}` from {file} must check:\n{out}");
            let _ = std::fs::remove_dir_all(&d);
        }
    }
}

/// The control: a match that leaves a variant out is still refused, for
/// every name and every shape.
#[test]
fn a_match_missing_a_variant_of_an_imported_enum_is_refused() {
    for (i, (file, path)) in ENUM_SHAPES.iter().enumerate() {
        for name in ENUM_NAMES {
            let d = tree(
                &format!("enum_missing_{i}_{name}"),
                &[(file, &enum_lib(name)), ("app/main.hl", &enum_app(path, name, &["Red"]))],
            );
            let (ok, out) = hale(&d.join("app"), &["check", "main.hl"]);
            assert!(!ok, "a match missing `Green` of `lib::{name}` from {file} must not check:\n{out}");
            assert!(out.contains("match is not exhaustive"), "the refusal names exhaustiveness:\n{out}");
            let _ = std::fs::remove_dir_all(&d);
        }
    }
}

/// The re-review's program, built and run: `Code_x` in a directory
/// library is merged under an encoded name, and the match reaches `red`.
#[test]
fn an_encoded_imported_enum_matches_natively() {
    let d = tree(
        "enum_native",
        &[("app/lib/defs.hl", &enum_lib("Code_x")), ("app/main.hl", &enum_app("lib", "Code_x", &["Red", "Green"]))],
    );
    let app = d.join("app");
    let s = symbols(&app.join("main.hl"));
    assert!(!s["lib::Code_x"].ends_with("_Code_x"), "the declaration's name is encoded: {}", s["lib::Code_x"]);
    let (ok, out) = hale(&app, &["run", "main.hl"]);
    assert!(ok, "run must pass:\n{out}");
    assert_eq!(out.trim(), "red");
    let _ = std::fs::remove_dir_all(&d);
}
