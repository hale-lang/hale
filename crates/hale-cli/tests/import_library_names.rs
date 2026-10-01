//! F.40 phase 3, C2 — a library's name is its own.
//!
//! Imported symbols are mangled under a name the load gives each library
//! (`AliasScopes::name_library`). Outside a workspace the name was the
//! library's file name, so two single-file libraries called `util.hl` in
//! different directories shared one symbol namespace: the second one's
//! declarations were merged under the first's mangled names. The load
//! now tells them apart, and a library that holds its name alone keeps
//! the name it always had.

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

/// Two single-file libraries with one file name, declaring the same
/// names, and an app that imports both: each alias answers with its own
/// library's value, and a type of one is not the other's.
#[test]
fn two_single_file_libraries_sharing_a_name_stay_apart() {
    let d = tree(
        "same_file_name",
        &[
            (
                "one/util.hl",
                "type Tag {\n    text: String;\n}\n\nfn who() -> String {\n    return \"one\";\n}\n",
            ),
            (
                "two/util.hl",
                "type Tag {\n    text: String;\n}\n\nfn who() -> String {\n    return \"two\";\n}\n",
            ),
            (
                "app/main.hl",
                "import \"../one/util\" as a;\n\
                 import \"../two/util\" as b;\n\
                 \n\
                 fn main() {\n\
                 \x20   let x = a::Tag { text: a::who() };\n\
                 \x20   let y = b::Tag { text: b::who() };\n\
                 \x20   println(x.text + \"/\" + y.text);\n\
                 }\n",
            ),
        ],
    );
    let app = d.join("app");
    let (ok, out) = hale(&app, &["check", "."]);
    assert!(ok, "check must pass:\n{out}");
    let (ok, out) = hale(&app, &["run", "."]);
    assert!(ok, "run must pass:\n{out}");
    assert_eq!(out.trim(), "one/two", "each alias reaches its own library");
    let _ = std::fs::remove_dir_all(&d);
}
