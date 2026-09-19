//! GH #714: a module alias and a free fn of the same name stay
//! distinct when the seed is IMPORTED.
//!
//! A seed may declare `import "../core" as core;` beside `fn
//! core(...)`: the alias governs `core::greet(...)` paths, the fn
//! governs `core("x")` calls, and the seed compiles and runs on its
//! own. Importing that seed used to break it — the mangler rewrote
//! every single- AND two-segment path head that named one of the
//! seed's own decls, so the ALIAS head in `core::greet` became the
//! free fn's mangled symbol and the per-build path-rename table
//! could no longer see it. `hale check` passed (it resolves the
//! alias) and `hale build` died with
//! `path call __lib_<id>_main_core::greet in expression position`.
//!
//! Covered here: the three-seed chain, a diamond where the importer
//! reaches both seeds, and a control seed whose free fn shadows
//! nothing (ordinary mangling must be untouched). Each case asserts
//! that the direct and the imported compile resolve the same
//! targets by running both.

use std::path::{Path, PathBuf};
use std::process::Command;

fn hale_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hale"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hale_i714_{}_{}",
        tag,
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn write(dir: &Path, rel: &str, src: &str) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, src).expect("write seed file");
}

fn check(seed: &Path) -> (bool, String) {
    let out = Command::new(hale_bin())
        .arg("check")
        .arg(seed)
        .output()
        .expect("invoke hale check");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (out.status.success(), text)
}

/// Build the seed directory and run the binary `hale build` drops
/// beside it. Returns (built_ok, combined build output, stdout).
fn build_and_run(seed: &Path) -> (bool, String, String) {
    let out = Command::new(hale_bin())
        .arg("build")
        .arg(seed)
        .output()
        .expect("invoke hale build");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        return (false, text, String::new());
    }
    let bin = seed.join(seed.file_name().expect("seed dir name"));
    let run = Command::new(&bin).output().expect("run built binary");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    if !run.status.success() {
        text.push_str(&String::from_utf8_lossy(&run.stderr));
        return (false, text, stdout);
    }
    (true, text, stdout)
}

const CORE: &str = r#"fn greet(name: String) -> String {
    return "hello " + name;
}
"#;

/// The library body under test: the alias `core` and the free fn
/// `core` coexist, and `go()` uses both.
const MID_BODY: &str = r#"import "../core" as core;

fn core(args: String) -> String {
    return "[" + args + "]";
}

fn go() -> String {
    return core::greet("mid") + " " + core("x");
}
"#;

#[test]
fn alias_and_same_named_free_fn_survive_import() {
    let dir = scratch("chain");
    write(&dir, "core/main.hl", CORE);
    write(&dir, "mid/main.hl", MID_BODY);
    write(
        &dir,
        "top/main.hl",
        "import \"../mid\" as mid;\n\nfn main() {\n    println(mid::go());\n}\n",
    );
    // The same body compiled DIRECTLY — the resolution both sides
    // must agree on.
    write(
        &dir,
        "direct/main.hl",
        &format!("{}\nfn main() {{\n    println(go());\n}}\n", MID_BODY),
    );

    let (direct_ok, direct_log, direct_out) = build_and_run(&dir.join("direct"));
    assert!(direct_ok, "direct build/run failed: {}", direct_log);
    assert_eq!(direct_out.trim(), "hello mid [x]", "direct: {:?}", direct_out);

    let (checked, check_log) = check(&dir.join("top"));
    assert!(checked, "hale check on the importer failed: {}", check_log);

    let (ok, log, stdout) = build_and_run(&dir.join("top"));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(ok, "importer build/run failed: {}", log);
    assert_eq!(
        stdout.trim(),
        "hello mid [x]",
        "the imported compile must resolve the same targets as the \
         direct one: {:?}",
        stdout
    );
}

#[test]
fn diamond_import_keeps_alias_and_free_fn_distinct() {
    // The importer reaches core BOTH directly and through mid, and
    // mid's alias for core collides with mid's own free fn. The
    // second visit registers the new alias against the cached
    // mangled names (#249); the head must still be the alias.
    let dir = scratch("diamond");
    write(&dir, "core/main.hl", CORE);
    write(&dir, "mid/main.hl", MID_BODY);
    write(
        &dir,
        "top/main.hl",
        "import \"../mid\" as mid;\nimport \"../core\" as core;\n\n\
         fn main() {\n    println(mid::go());\n    \
         println(core::greet(\"top\"));\n}\n",
    );

    let (checked, check_log) = check(&dir.join("top"));
    assert!(checked, "hale check on the diamond importer failed: {}", check_log);

    let (ok, log, stdout) = build_and_run(&dir.join("top"));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(ok, "diamond build/run failed: {}", log);
    assert!(
        stdout.contains("hello mid [x]"),
        "expected mid::go() output: {:?}",
        stdout
    );
    assert!(
        stdout.contains("hello top"),
        "expected the importer's own core::greet output: {:?}",
        stdout
    );
}

#[test]
fn free_fn_shadowing_nothing_still_mangles() {
    // Control: no collision. The imported seed's own free fn is
    // still renamed and still reachable through its call sites, and
    // the alias head still resolves — i.e. the #714 exemption did
    // not disable ordinary mangling.
    let dir = scratch("control");
    write(&dir, "core/main.hl", CORE);
    write(
        &dir,
        "mid/main.hl",
        "import \"../core\" as core;\n\n\
         fn decorate(args: String) -> String {\n    \
         return \"[\" + args + \"]\";\n}\n\n\
         fn go() -> String {\n    \
         return core::greet(\"mid\") + \" \" + decorate(\"x\");\n}\n",
    );
    write(
        &dir,
        "top/main.hl",
        "import \"../mid\" as mid;\n\nfn main() {\n    println(mid::go());\n}\n",
    );

    let (checked, check_log) = check(&dir.join("top"));
    assert!(checked, "hale check failed: {}", check_log);

    let (ok, log, stdout) = build_and_run(&dir.join("top"));
    let bin_path = dir.join("top").join("top");
    let bin_bytes = std::fs::read(&bin_path).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(ok, "control build/run failed: {}", log);
    assert_eq!(stdout.trim(), "hello mid [x]", "control: {:?}", stdout);
    let needle = b"_main_decorate";
    assert!(
        bin_bytes.windows(needle.len()).any(|w| w == needle),
        "the imported seed's free fn should still carry a mangled \
         `__lib_<id>_main_decorate` symbol"
    );
}
