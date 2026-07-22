//! iris friction F.10 (GH #249): diamond imports — a lib reached
//! a SECOND time (by the entry AND by another lib, or by two
//! libs) must still register the second importer's alias against
//! the shared mangled names. Pre-fix, the resolver's visited-set
//! dedup skipped the alias registration entirely on revisit, so
//! every `alias::Name` in the second importer leaked unrenamed
//! into codegen: `hale check` passed (imports aren't
//! deep-checked) and `hale build` died with "qualified type
//! `g::Rect` not in stdlib path-renames table" (or the mangled
//! unknown-type-in-signature variant). The iris shape: iris and
//! lib/lotus_viz both import lib/raylib.

use std::path::PathBuf;
use std::process::Command;

fn write_project(order_viz_first: bool) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hale_diamond_{}_{}",
        if order_viz_first { "vf" } else { "gf" },
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lib/geom")).expect("mkdir");
    std::fs::create_dir_all(dir.join("lib/viz")).expect("mkdir");
    std::fs::write(
        dir.join("lib/geom/types.hl"),
        r#"type Rect {
    x: Float = 0.0;
    w: Float = 0.0;
}
"#,
    )
    .expect("write geom");
    // viz reaches geom under its OWN alias `g`, and uses it in a
    // fn SIGNATURE — the position that leaked.
    std::fs::write(
        dir.join("lib/viz/draw.hl"),
        r#"import "../geom" as g;

fn area(r: g::Rect) -> Float {
    return r.x * r.w;
}
"#,
    )
    .expect("write viz");
    // Both import orders exercised: whichever resolves second
    // hits the visited-dedup path.
    let imports = if order_viz_first {
        "import \"lib/viz\" as viz;\nimport \"lib/geom\" as geo;"
    } else {
        "import \"lib/geom\" as geo;\nimport \"lib/viz\" as viz;"
    };
    std::fs::write(
        dir.join("main.hl"),
        format!(
            r#"{}

type Panel {{
    bounds: geo::Rect;
}}

fn pick() -> geo::Rect {{
    return geo::Rect {{ x: 2.0, w: 3.0 }};
}}

fn main() {{
    let r = pick();
    let p = Panel {{ bounds: r }};
    println(viz::area(p.bounds));
}}
"#,
            imports
        ),
    )
    .expect("write main");
    dir
}

fn build_and_run(dir: &PathBuf) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_hale"))
        .arg("build")
        .arg(dir)
        .output()
        .expect("hale build");
    if !out.status.success() {
        return (
            false,
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        );
    }
    let bin = dir.join(dir.file_name().unwrap());
    let run = Command::new(&bin).output().expect("run built binary");
    (
        run.status.success(),
        String::from_utf8_lossy(&run.stdout).into_owned(),
        String::from_utf8_lossy(&run.stderr).into_owned(),
    )
}

#[test]
fn diamond_import_geom_first_builds_and_runs() {
    let dir = write_project(false);
    let (ok, stdout, stderr) = build_and_run(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(ok, "stdout: {}\nstderr: {}", stdout, stderr);
    assert!(stdout.contains('6'), "got: {:?}", stdout);
}

#[test]
fn diamond_import_viz_first_builds_and_runs() {
    let dir = write_project(true);
    let (ok, stdout, stderr) = build_and_run(&dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(ok, "stdout: {}\nstderr: {}", stdout, stderr);
    assert!(stdout.contains('6'), "got: {:?}", stdout);
}
