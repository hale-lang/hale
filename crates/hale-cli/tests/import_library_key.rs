//! GH #763 — one library under two spellings is ONE library.
//!
//! A seed is a directory (F.19): every `.hl` file in it shares one
//! declaration namespace and `main.hl` is that seed's entry file. So
//! `import "../lib"` and `import "../lib/main"` name the same
//! library, and an app whose files use one spelling each must see one
//! library through both aliases.
//!
//! It did not. `resolve_imports` derived the library's identity from
//! the import TARGET — the canonicalized directory for a directory
//! hit, the canonicalized file for a single-file hit — so the two
//! spellings produced two `lib_key`s, two `lib_id`s and two sets of
//! mangled symbols. The `visited` set that stops a file being parsed
//! twice is global across the build, so whichever spelling resolved
//! second found every file already parsed, registered no rename rows
//! under its own key, and every `alias::Name` written against it died
//! at codegen as `unknown qualified name` — while the other alias
//! worked and `hale check` said `ok`.
//!
//! The rule now: a single-file hit that is a seed's `main.hl`
//! resolves to that seed's DIRECTORY. Any other single file is still
//! its own library (resolution-order rule 1), which the last test
//! pins.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Write a throwaway multi-seed tree under a pid-unique temp dir.
fn seed(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let d: PathBuf = std::env::temp_dir().join(format!(
        "hale_library_key_{}_{}",
        std::process::id(),
        tag
    ));
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
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

/// The library seed: two files, so the directory spelling and the
/// entry-file spelling would disagree about the file set as well as
/// about the identity.
const LIB_MAIN: &str = "type Greeting {\n\
                        \x20   text: String;\n\
                        }\n\
                        \n\
                        fn hello() -> String {\n\
                        \x20   return \"lib-hello\";\n\
                        }\n";

const LIB_HELPER: &str = "fn helper_name() -> String {\n\
                          \x20   return \"lib-helper\";\n\
                          }\n";

/// check (text and `--json`), build and run, all green, on a
/// directory seed. Returns `run`'s output lines.
fn green(app: &Path) -> Vec<String> {
    let (ok, out) = hale(app, &["check", "."]);
    assert!(ok, "check must pass:\n{out}");
    let (ok, out) = hale(app, &["check", "--json", "."]);
    assert!(ok, "check --json must pass:\n{out}");
    assert!(
        !out.contains("\"severity\""),
        "check --json must report no diagnostics:\n{out}"
    );
    // The symbol identity only bites at codegen/link, so BUILD, not
    // just check.
    let (ok, out) = hale(app, &["build", "."]);
    assert!(ok, "build must pass:\n{out}");
    let (ok, out) = hale(app, &["run", "."]);
    assert!(ok, "run must pass:\n{out}");
    out.lines().map(|l| l.to_string()).collect()
}

/// The issue's shape: the directory spelling in one file of the app
/// seed, the entry-file spelling in another. Both aliases must
/// resolve, the entry-file spelling must see the WHOLE seed
/// (`helper.hl` too), and a value built through one alias must be
/// accepted where the other alias's type is declared — which is only
/// true if both mangle to one set of symbols.
#[test]
fn directory_and_entry_file_spellings_are_one_library() {
    let d = seed(
        "both",
        &[
            ("lib/main.hl", LIB_MAIN),
            ("lib/helper.hl", LIB_HELPER),
            (
                "app/main.hl",
                "import \"../lib\" as a;\n\
                 \n\
                 fn main() {\n\
                 \x20   println(a::hello());\n\
                 \x20   println(from_other());\n\
                 \x20   println(shout(a::Greeting { text: \"cross\" }));\n\
                 }\n",
            ),
            (
                "app/other.hl",
                "import \"../lib/main\" as b;\n\
                 \n\
                 fn from_other() -> String {\n\
                 \x20   let g = b::Greeting { text: b::hello() };\n\
                 \x20   return g.text + \"/\" + b::helper_name();\n\
                 }\n\
                 \n\
                 fn shout(g: b::Greeting) -> String {\n\
                 \x20   return \"!\" + g.text;\n\
                 }\n",
            ),
        ],
    );
    assert_eq!(
        green(&d.join("app")),
        vec!["lib-hello", "lib-hello/lib-helper", "!cross"],
        "both aliases answer, the entry-file spelling reaches \
         helper.hl, and a::Greeting is b::Greeting",
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// The other resolution order: the entry-file spelling resolves
/// FIRST (the app's files are merged in alphabetical order), so it is
/// the directory spelling that arrives to find the library already
/// parsed. Pre-fix this failed the other way round — the directory
/// alias saw only the files the file spelling had not taken.
#[test]
fn either_spelling_may_resolve_first() {
    let d = seed(
        "order",
        &[
            ("lib/main.hl", LIB_MAIN),
            ("lib/helper.hl", LIB_HELPER),
            (
                "app/main.hl",
                "import \"../lib/main\" as b;\n\
                 \n\
                 fn main() {\n\
                 \x20   println(b::hello());\n\
                 \x20   println(from_other());\n\
                 \x20   println(shout(b::Greeting { text: \"cross\" }));\n\
                 }\n",
            ),
            (
                "app/other.hl",
                "import \"../lib\" as a;\n\
                 \n\
                 fn from_other() -> String {\n\
                 \x20   let g = a::Greeting { text: a::hello() };\n\
                 \x20   return g.text + \"/\" + a::helper_name();\n\
                 }\n\
                 \n\
                 fn shout(g: a::Greeting) -> String {\n\
                 \x20   return \"!\" + g.text;\n\
                 }\n",
            ),
        ],
    );
    assert_eq!(
        green(&d.join("app")),
        vec!["lib-hello", "lib-hello/lib-helper", "!cross"],
        "the spelling that resolves second must register its rows \
         whichever one it is",
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// Control: two aliases, ONE spelling, across two files of the app
/// seed. This has worked since GH #746 seeded the per-library rename
/// cache; it is here so a regression in that path is told apart from
/// the two-spelling one.
#[test]
fn two_aliases_one_spelling_still_works() {
    let d = seed(
        "twoalias",
        &[
            ("lib/main.hl", LIB_MAIN),
            ("lib/helper.hl", LIB_HELPER),
            (
                "app/main.hl",
                "import \"../lib\" as a;\n\
                 \n\
                 fn main() {\n\
                 \x20   println(a::hello());\n\
                 \x20   println(from_other());\n\
                 }\n",
            ),
            (
                "app/other.hl",
                "import \"../lib\" as c;\n\
                 \n\
                 fn from_other() -> String {\n\
                 \x20   return c::hello() + \"/\" + c::helper_name();\n\
                 }\n",
            ),
        ],
    );
    assert_eq!(
        green(&d.join("app")),
        vec!["lib-hello", "lib-hello/lib-helper"],
        "one spelling under two aliases is the control",
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// Control: the entry-file spelling ALONE. It resolves, and — being
/// a name for the seed — it reaches every file of that seed, not just
/// `main.hl`.
#[test]
fn entry_file_spelling_alone_names_the_whole_seed() {
    let d = seed(
        "filealone",
        &[
            ("lib/main.hl", LIB_MAIN),
            ("lib/helper.hl", LIB_HELPER),
            (
                "app/main.hl",
                "import \"../lib/main\" as b;\n\
                 \n\
                 fn main() {\n\
                 \x20   let g = b::Greeting { text: b::hello() };\n\
                 \x20   println(g.text + \"/\" + b::helper_name());\n\
                 }\n",
            ),
        ],
    );
    assert_eq!(
        green(&d.join("app")),
        vec!["lib-hello/lib-helper"],
        "`lib/main` is `lib`",
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// The transitive shape: the app reaches the library under the
/// directory spelling and a MIDDLE library reaches the same library
/// under the entry-file spelling. The two importers are different
/// seeds, so nothing here is about one seed's alias namespace — it is
/// purely the library's identity.
#[test]
fn app_and_another_library_may_spell_it_differently() {
    let d = seed(
        "transitive",
        &[
            ("lib/main.hl", LIB_MAIN),
            ("lib/helper.hl", LIB_HELPER),
            (
                "mid/main.hl",
                "import \"../lib/main\" as m;\n\
                 \n\
                 fn wrapped() -> String {\n\
                 \x20   return m::hello() + \"/\" + m::helper_name();\n\
                 }\n\
                 \n\
                 fn shout(g: m::Greeting) -> String {\n\
                 \x20   return \"!\" + g.text;\n\
                 }\n",
            ),
            (
                "app/main.hl",
                "import \"../lib\" as a;\n\
                 import \"../mid\" as mid;\n\
                 \n\
                 fn main() {\n\
                 \x20   println(a::hello());\n\
                 \x20   println(mid::wrapped());\n\
                 \x20   println(mid::shout(a::Greeting { text: \"cross\" }));\n\
                 }\n",
            ),
        ],
    );
    assert_eq!(
        green(&d.join("app")),
        vec!["lib-hello", "lib-hello/lib-helper", "!cross"],
        "the app's a::Greeting is the mid library's m::Greeting",
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// GH #820: the same single file, when ANOTHER import of the build
/// takes the directory around it. The two identities are genuinely
/// different libraries — one file against the whole seed — and
/// `visited` is global, so whichever resolved second got only the
/// files the first had not taken and one alias's names silently
/// resolved to nothing. The ruling (2026-09-20, GH #911): refuse the
/// FILE spelling, naming the library that holds the file and where
/// that import is written.
///
/// The directory spelling resolves first here (the app's files merge
/// alphabetically, so `main.hl`'s imports come before `other.hl`'s).
#[test]
fn a_file_of_a_directory_imported_library_is_refused() {
    let d = seed(
        "conflict",
        &[
            ("lib/main.hl", LIB_MAIN),
            ("lib/helper.hl", LIB_HELPER),
            (
                "app/main.hl",
                "import \"../lib\" as a;\n\
                 \n\
                 fn main() {\n\
                 \x20   println(a::hello());\n\
                 \x20   println(from_other());\n\
                 }\n",
            ),
            (
                "app/other.hl",
                "import \"../lib/helper\" as h;\n\
                 \n\
                 fn from_other() -> String {\n\
                 \x20   return h::helper_name();\n\
                 }\n",
            ),
        ],
    );
    let app = d.join("app");
    for cmd in [&["check", "."][..], &["build", "."][..]] {
        let (ok, out) = hale(&app, cmd);
        assert!(!ok, "{:?} must fail:\n{out}", cmd);
        assert!(
            out.contains(
                "`../lib/helper` is already part of the library imported \
                 as `a` at"
            ),
            "the message names the file spelling and the library that \
             holds it:\n{out}"
        );
        // Located at the import that has to change — `other.hl`'s,
        // under the path literal.
        assert!(
            out.contains("other.hl:1:8:"),
            "located at the refused import:\n{out}"
        );
        // ... naming where the other one is written.
        assert!(out.contains("main.hl:1;"), "names the first site:\n{out}");
    }
    let (ok, out) = hale(&app, &["check", "--json", "."]);
    assert!(!ok, "{out}");
    let line = out
        .lines()
        .find(|l| l.contains("is already part of the library"))
        .unwrap_or_else(|| panic!("no json row for the conflict:\n{out}"));
    assert!(line.contains("other.hl\""), "{line}");
    assert!(line.contains("\"line\":1"), "{line}");
    assert!(line.contains("\"col\":8"), "{line}");
    assert!(line.contains("\"severity\":\"error\""), "{line}");
    let _ = std::fs::remove_dir_all(&d);
}

/// The other resolution order: the single FILE resolves first and the
/// directory arrives to find its files taken. The refusal is the same
/// one, still located at the file spelling — that is the spelling
/// being refused, and pointing at the directory import instead would
/// name a line there is nothing wrong with.
#[test]
fn either_order_refuses_the_file_spelling() {
    let d = seed(
        "conflict_order",
        &[
            ("lib/main.hl", LIB_MAIN),
            ("lib/helper.hl", LIB_HELPER),
            (
                "app/main.hl",
                "import \"../lib/helper\" as h;\n\
                 \n\
                 fn main() {\n\
                 \x20   println(h::helper_name());\n\
                 \x20   println(from_other());\n\
                 }\n",
            ),
            (
                "app/other.hl",
                "import \"../lib\" as a;\n\
                 \n\
                 fn from_other() -> String {\n\
                 \x20   return a::hello();\n\
                 }\n",
            ),
        ],
    );
    let app = d.join("app");
    let (ok, out) = hale(&app, &["check", "."]);
    assert!(!ok, "check must fail:\n{out}");
    assert!(
        out.contains(
            "`../lib/helper` is already part of the library imported as \
             `a` at"
        ),
        "{out}"
    );
    assert!(
        out.contains("main.hl:1:8:"),
        "the file spelling is in main.hl this time:\n{out}"
    );
    assert!(out.contains("other.hl:1;"), "names the first site:\n{out}");
    let _ = std::fs::remove_dir_all(&d);
}

/// A TRANSITIVE conflict: the app takes the directory and a middle
/// library takes one of its files. Two different importing seeds, so
/// nothing here is about one seed's alias namespace — it is the
/// library's identity, and the refusal is located in the middle
/// library's own file.
#[test]
fn a_conflict_across_two_importers_is_refused() {
    let d = seed(
        "conflict_transitive",
        &[
            ("lib/main.hl", LIB_MAIN),
            ("lib/helper.hl", LIB_HELPER),
            (
                "mid/main.hl",
                "import \"../lib/helper\" as m;\n\
                 \n\
                 fn wrapped() -> String {\n\
                 \x20   return m::helper_name();\n\
                 }\n",
            ),
            (
                "app/main.hl",
                "import \"../lib\" as a;\n\
                 import \"../mid\" as mid;\n\
                 \n\
                 fn main() {\n\
                 \x20   println(a::hello());\n\
                 \x20   println(mid::wrapped());\n\
                 }\n",
            ),
        ],
    );
    let app = d.join("app");
    let (ok, out) = hale(&app, &["check", "."]);
    assert!(!ok, "check must fail:\n{out}");
    assert!(
        out.contains(
            "`../lib/helper` is already part of the library imported as \
             `a` at"
        ),
        "{out}"
    );
    assert!(
        out.contains("mid/main.hl:1:8:"),
        "located where the file spelling is written:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// The rule's boundary: only `main.hl` is a second name for its
/// directory. A single file that is NOT a seed's entry file stays the
/// single-file library of resolution-order rule 1 — it brings in that
/// file and nothing else.
#[test]
fn a_non_entry_single_file_is_still_its_own_library() {
    let d = seed(
        "singlefile",
        &[
            ("lib/main.hl", LIB_MAIN),
            ("lib/helper.hl", LIB_HELPER),
            (
                "app/main.hl",
                "import \"../lib/helper\" as h;\n\
                 \n\
                 fn main() {\n\
                 \x20   println(h::helper_name());\n\
                 }\n",
            ),
        ],
    );
    let app = d.join("app");
    assert_eq!(
        green(&app),
        vec!["lib-helper"],
        "the single-file library resolves",
    );
    // ... and it is only that file: `main.hl`'s decls are not in it.
    std::fs::write(
        app.join("main.hl"),
        "import \"../lib/helper\" as h;\n\
         \n\
         fn main() {\n\
         \x20   println(h::hello());\n\
         }\n",
    )
    .expect("rewrite");
    let (ok, out) = hale(&app, &["build", "."]);
    assert!(
        !ok,
        "`hello` lives in lib/main.hl, which this library is not:\n{out}"
    );
    assert!(
        out.contains("h::hello"),
        "the failure must name the path that does not resolve:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}
