//! GH #1951 item 6 (F.51): the refusals of an import list, on the verbs
//! a user runs, located at the item that has to change.
//!
//! - a name the imported seed neither declares nor re-exports;
//! - a bare name that is also a declaration of the importing seed;
//! - a bare name two lists bind to two declarations (one declaration
//!   reached under two spellings is one name, and is not refused);
//! - an import list on an import of a file of the seed itself, which is
//!   refused as any such import is: its names are already bare.
//!
//! The lists that build are `hale-codegen`'s `import_lists.rs`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn seeds(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let d: PathBuf = std::env::temp_dir().join(format!("hale_import_lists_{}_{}", std::process::id(), tag));
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
        .env("HALE_SKIP_STALE_CHECK", "1")
        .output()
        .expect("hale");
    (
        out.status.success(),
        format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)),
    )
}

const STORE: &str = "type ModelRow { id: Int; }\nfn count(r: ModelRow) -> Int { return r.id; }\ngroup g = { count };\n";
const SOURCE: &str = "type Beat { n: Int; }\n";
const API: &str = "import \"../store\" as st { ModelRow };\nfn make() -> ModelRow { return ModelRow { id: 1 }; }\n";

/// An item the imported seed does not declare is refused at the item,
/// with what the library does provide; so is a top-level name that is
/// no listable declaration (a group).
#[test]
fn an_unknown_item_is_refused_at_the_item() {
    let d = seeds(
        "unknown",
        &[
            ("store/store.hl", STORE),
            ("app/main.hl", "import \"../store\" as st { ModelRow, Nope, g };\nfn main() { }\n"),
        ],
    );
    for verb in ["check", "build"] {
        let (ok, out) = hale(&d.join("app"), &[verb, "."]);
        assert!(!ok, "`hale {verb}` refuses the list:\n{out}");
        assert!(
            out.contains(
                "main.hl:1:37: type error: `Nope` is not a top-level name of the library imported as `st` \
                 (\"../store\"): an import list names a type, locus, fn, const, topic or interface \
                 that library declares or re-exports; `st` provides: ModelRow, count"
            ),
            "`hale {verb}`: the item, located, with what `st` provides:\n{out}"
        );
        assert!(out.contains("main.hl:1:43: type error: `g` is not a top-level name"), "`hale {verb}`: a group is not listable:\n{out}");
        assert!(!out.contains("`ModelRow` is not"), "a declared name is not refused:\n{out}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// A listed name that the importing seed also declares — in any of its
/// files — is refused at the item, naming the declaration.
#[test]
fn a_listed_name_colliding_with_a_declaration_is_refused() {
    let d = seeds(
        "decl",
        &[
            ("store/store.hl", STORE),
            ("app/main.hl", "import \"../store\" as st { ModelRow, count };\nfn main() { }\n"),
            ("app/rows.hl", "type ModelRow { name: String; }\n"),
        ],
    );
    let (ok, out) = hale(&d.join("app"), &["check", "."]);
    assert!(!ok, "check refuses the collision:\n{out}");
    assert!(
        out.contains("main.hl:1:27: type error: `ModelRow` is listed on the import of `st`, and this seed declares `ModelRow` at ")
            && out.contains("rows.hl:1: a bare name names one declaration — rename the import (`ModelRow as StModelRow`) or the declaration"),
        "the item, naming the declaration and a rename:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// Two lists binding one bare name to two declarations: refused at the
/// second, naming the first. One declaration under two spellings — `st`'s
/// own `ModelRow` and `api`'s re-export of it — is one name.
#[test]
fn two_lists_binding_one_name_to_two_declarations_are_refused() {
    let d = seeds(
        "two",
        &[
            ("store/store.hl", STORE),
            ("source/source.hl", SOURCE),
            ("api/api.hl", API),
            (
                "app/main.hl",
                "import \"../store\" as st { ModelRow };\n\
                 import \"../api\" as api { ModelRow };\n\
                 import \"../source\" as so { Beat as ModelRow };\n\
                 fn main() { }\n",
            ),
        ],
    );
    let (ok, out) = hale(&d.join("app"), &["check", "."]);
    assert!(!ok, "check refuses the collision:\n{out}");
    assert!(
        out.contains(
            "main.hl:3:36: type error: `ModelRow` is listed on the import of `so` and on the import of `st` at "
        ) && out.contains("main.hl:1, naming two declarations: a bare name names one — rename one of them (`Beat as SoBeat`)"),
        "the second binding, naming the first:\n{out}"
    );
    assert!(!out.contains("main.hl:2:"), "`api`'s re-export of the same declaration is not refused:\n{out}");

    let d2 = seeds(
        "same",
        &[
            ("store/store.hl", STORE),
            ("api/api.hl", API),
            (
                "app/main.hl",
                "import \"../store\" as st { ModelRow, count };\n\
                 import \"../api\" as api { ModelRow };\n\
                 fn main() { println(to_string(count(api::make()) + count(ModelRow { id: 2 }))); }\n",
            ),
        ],
    );
    let (ok, out) = hale(&d2.join("app"), &["run", "."]);
    assert!(ok && out.lines().any(|l| l == "3"), "one declaration under two spellings runs:\n{out}");
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(&d2);
}

/// A list on an import of the seed's own file is refused as the import
/// of a file of the seed always is, at the path: the names are bare
/// already, without a list.
#[test]
fn a_list_on_an_import_of_the_seeds_own_file_is_refused() {
    let d = seeds(
        "own",
        &[
            ("app/main.hl", "import \"sign\" as sign { verify };\nfn main() { println(to_string(verify(2))); }\n"),
            ("app/sign.hl", "fn verify(x: Int) -> Int { return x * 2; }\n"),
        ],
    );
    let (ok, out) = hale(&d.join("app"), &["check", "."]);
    assert!(!ok, "check refuses the import:\n{out}");
    assert!(
        out.contains("main.hl:1:8: type error: `sign` names sign.hl, a file of this seed:")
            && out.contains("its declarations are already in scope by their bare names"),
        "located at the path:\n{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}
