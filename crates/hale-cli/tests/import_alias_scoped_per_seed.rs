//! GH #746 — an import alias is scoped to the seed that declares it,
//! in the build as in the language.
//!
//! The per-build path-rename table is keyed by the alias as written
//! (`["u", "f"] -> __lib_<lib>_main_f`), so two seeds that bind the
//! same alias name to DIFFERENT libraries collided in it: the last row
//! pushed won and BOTH seeds' `u::f()` resolved to one library. The
//! reduction is four seeds:
//!
//! - `libx` / `liby` each declare `fn f()`,
//! - `a` imports `../libx` as `u` and calls `u::f()`,
//! - `top` imports `../a` as `a` AND `../liby` as `u`.
//!
//! `hale check` answered `ok`, and the binary printed `from-y` twice —
//! seed `a`'s call resolved to `top`'s library. Silent wrong code with
//! no diagnostic anywhere.
//!
//! Aliases are file/seed-scoped in the language (spec `projects.md`,
//! "Scoped imports (A4)"), so the fix scopes the table the same way:
//! each binder of a contested alias gets a head of its own and its own
//! references are re-headed to match.

use std::path::{Path, PathBuf};
use std::process::Command;

fn seed(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let d: PathBuf = std::env::temp_dir().join(format!(
        "hale_alias_scope_{}_{}",
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

/// `hale build <dir>` then run the produced binary (the directory
/// entry path — `run`'s single-file path is exercised directly).
fn build_and_run(dir: &Path) -> (bool, String) {
    let (ok, out) = hale(dir, &["build", "."]);
    if !ok {
        return (false, out);
    }
    let bin = dir.join(
        dir.canonicalize()
            .unwrap_or_else(|_| dir.to_path_buf())
            .file_name()
            .expect("dir name"),
    );
    let run = Command::new(&bin).output().expect("run built binary");
    (
        run.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        ),
    )
}

/// The issue's four-seed reduction: `from-x` then `from-y`, because
/// each seed's `u` is its own.
#[test]
fn two_seeds_binding_one_alias_to_two_libs_keep_their_own() {
    let d = seed(
        "two_libs",
        &[
            ("libx/main.hl", "fn f() -> String { return \"from-x\"; }\n"),
            ("liby/main.hl", "fn f() -> String { return \"from-y\"; }\n"),
            (
                "a/main.hl",
                "import \"../libx\" as u;\n\
                 fn via_a() -> String { return u::f(); }\n",
            ),
            (
                "top/main.hl",
                "import \"../a\" as a;\n\
                 import \"../liby\" as u;\n\
                 fn main() {\n\
                 \x20   println(a::via_a());\n\
                 \x20   println(u::f());\n\
                 }\n",
            ),
        ],
    );
    let top = d.join("top");

    let (ok, out) = hale(&top, &["check", "."]);
    assert!(ok, "check: {out}");

    // The single-file entry path (`hale run main.hl`), the issue's
    // own command.
    let (ok, out) = hale(&top, &["run", "main.hl"]);
    assert!(ok, "run: {out}");
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        vec!["from-x", "from-y"],
        "each seed's `u` resolves to the lib that seed imported: {out}"
    );

    // The directory entry path resolves it the same way.
    let (ok, out) = build_and_run(&top);
    assert!(ok, "build+run: {out}");
    assert_eq!(out.lines().collect::<Vec<_>>(), vec!["from-x", "from-y"], "{out}");

    let _ = std::fs::remove_dir_all(&d);
}

/// The SAME alias for the SAME lib in two seeds is not a conflict —
/// both rows say the same thing, and the alias keeps its plain head
/// (the shape every multi-seed program in the corpus has, down to the
/// many seeds that each say `as dna` for one library).
#[test]
fn one_alias_for_one_lib_in_two_seeds_still_resolves() {
    let d = seed(
        "one_lib",
        &[
            ("lib/main.hl", "fn f() -> String { return \"shared\"; }\n"),
            (
                "a/main.hl",
                "import \"../lib\" as u;\n\
                 fn via_a() -> String { return u::f(); }\n",
            ),
            (
                "top/main.hl",
                "import \"../a\" as a;\n\
                 import \"../lib\" as u;\n\
                 fn main() {\n\
                 \x20   println(a::via_a());\n\
                 \x20   println(u::f());\n\
                 }\n",
            ),
        ],
    );
    let top = d.join("top");
    let (ok, out) = hale(&top, &["check", "."]);
    assert!(ok, "check: {out}");
    let (ok, out) = hale(&top, &["run", "main.hl"]);
    assert!(ok, "run: {out}");
    assert_eq!(out.lines().collect::<Vec<_>>(), vec!["shared", "shared"], "{out}");
    let _ = std::fs::remove_dir_all(&d);
}

/// Three files, two aliases crossing, in every position a qualified
/// path can hold: a call, a const value, a struct literal, and a type
/// in a signature. `mid`'s `u` is `libx`, `top`'s `u` is `liby`, and
/// `top` reaches `mid` as `m`.
#[test]
fn crossing_aliases_resolve_in_every_path_position() {
    let lib = |tag: &str, val: &str| {
        format!(
            "type Box {{ v: Int; }}\n\
             const TAG: String = \"{tag}\";\n\
             fn f() -> String {{ return \"{val}\"; }}\n\
             fn unbox(b: Box) -> Int {{ return b.v; }}\n"
        )
    };
    let d = seed(
        "crossing",
        &[
            ("libx/main.hl", &lib("x-tag", "from-x")),
            ("liby/main.hl", &lib("y-tag", "from-y")),
            (
                "mid/main.hl",
                "import \"../libx\" as u;\n\
                 \n\
                 fn mid_call() -> String { return u::f(); }\n\
                 fn mid_const() -> String { return u::TAG; }\n\
                 fn mid_lit() -> Int { return u::unbox(u::Box { v: 11 }); }\n",
            ),
            (
                "top/main.hl",
                "import \"../mid\" as m;\n\
                 import \"../liby\" as u;\n\
                 \n\
                 fn top_ty(b: u::Box) -> Int { return b.v; }\n\
                 \n\
                 fn main() {\n\
                 \x20   println(m::mid_call(), \" \", u::f());\n\
                 \x20   println(m::mid_const(), \" \", u::TAG);\n\
                 \x20   println(m::mid_lit(), \" \", u::unbox(u::Box { v: 22 }));\n\
                 \x20   println(top_ty(u::Box { v: 44 }));\n\
                 }\n",
            ),
        ],
    );
    let top = d.join("top");
    let (ok, out) = hale(&top, &["check", "."]);
    assert!(ok, "check: {out}");
    let (ok, out) = build_and_run(&top);
    assert!(ok, "build+run: {out}");
    assert_eq!(
        out.lines().collect::<Vec<_>>(),
        vec!["from-x from-y", "x-tag y-tag", "11 22", "44"],
        "{out}"
    );
    let _ = std::fs::remove_dir_all(&d);
}
